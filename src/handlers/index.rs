//! `GET /` — the dashboard. Aggregates fixtures, predictions, leaderboard,
//! group tables and admin tools into a single Askama render.

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};

use uuid::Uuid;

use crate::auth::MaybeAuthenticatedUser;
use crate::badges;
use crate::handlers::services::{
    build_badge_context, fetch_actual_champion, fetch_group_standings, fetch_leaderboard,
};
use crate::handlers::util::{flag_url, format_kickoff, render_template};
use crate::news;
use crate::scoring;
use crate::scoring::MatchScoringSystem;
use crate::translations::T;
use crate::views::{
    ChampPrediction, GroupStandingsTable, LeaderboardEntry, MatchView, SpecialPredictionsView,
    StageGroups, TeamView, UserPrediction,
};
use crate::AppState;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    user_name: String,
    user_total_points: i32,
    user_rank: usize,
    tipprunden_name: String,
    default_tab: String,
    started_in_progress: StageGroups,
    started_finished: StageGroups,
    open_matches: StageGroups,
    open_count: usize,
    next_deadline_iso: Option<String>,
    started_count: usize,
    leaderboard: Vec<LeaderboardEntry>,
    group_standings: Vec<GroupStandingsTable>,
    team_options: Vec<TeamView>,
    special_preds: SpecialPredictionsView,
    tournament_locked: bool,
    champ_preds: Vec<ChampPrediction>,
    is_admin: bool,
    can_create_league: bool,
    league_id: Uuid,
    league_name: String,
    news_items: Vec<news::NewsItem>,
    badges: Vec<badges::BadgeView>,
    t: T,
    lang_code: String,
    dev_mode: bool,
}

pub async fn index(
    State(state): State<AppState>,
    MaybeAuthenticatedUser(maybe_user): MaybeAuthenticatedUser,
) -> Result<Response, crate::handlers::util::HandlerError> {
    let user = match maybe_user {
        Some(u) => u,
        None => {
            let count = state.repos.users.count().await.map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            })?;
            if count == 0 {
                return Ok(Redirect::to("/setup").into_response());
            }
            return Err((
                StatusCode::UNAUTHORIZED,
                crate::handlers::util::t_for(&state, "de").get("error-not-authenticated"),
            ));
        }
    };
    let now = crate::time::now(&state.mock_now);

    let rows = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .unwrap_or_default();

    let other_preds_rows = state
        .repos
        .predictions
        .list_other_users_locked(user.id, user.league_id, now)
        .await
        .unwrap_or_default();

    let mut preds_by_match: std::collections::HashMap<i32, Vec<(String, i32, i32)>> =
        std::collections::HashMap::new();
    for p in other_preds_rows {
        preds_by_match.entry(p.match_id).or_default().push((
            p.user_name,
            p.predicted_home,
            p.predicted_away,
        ));
    }

    let league_config = state
        .repos
        .leagues
        .get_config(user.league_id)
        .await
        .unwrap_or_default();

    let first_kickoff = if league_config.predict_knockout_only {
        state
            .repos
            .matches
            .first_knockout_kickoff()
            .await
            .unwrap_or_default()
    } else {
        state
            .repos
            .matches
            .first_kickoff()
            .await
            .unwrap_or_default()
    };
    let tournament_locked = first_kickoff.is_some_and(|dt| dt < now);

    let special_preds = SpecialPredictionsView {
        champion_id: state
            .repos
            .special_predictions
            .get_user_champion(user.id)
            .await
            .unwrap_or_default(),
    };

    let team_options: Vec<TeamView> = state
        .repos
        .teams
        .list_real_for_dropdown()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| TeamView {
            id: t.id,
            name: t.name,
        })
        .collect();

    let champ_preds: Vec<ChampPrediction> = if tournament_locked {
        let actual = fetch_actual_champion(&state.repos).await;
        state
            .repos
            .special_predictions
            .list_with_user_names(user.league_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.champion_id.is_some())
            .map(|r| {
                let pts = match (r.champion_id, actual) {
                    (Some(p), Some(a)) => Some(if p == a { 10 } else { 0 }),
                    _ => None,
                };
                ChampPrediction {
                    name: r.user_name,
                    team_name: r.team_name.unwrap_or_default(),
                    team_flag: flag_url(&r.flag_code),
                    points: pts,
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut started_in_progress = StageGroups::default();
    let mut started_finished = StageGroups::default();
    let mut open_matches = StageGroups::default();
    let mut next_deadline: Option<chrono::DateTime<chrono::Utc>> = None;

    for r in rows {
        if r.team_home_id.is_none() || r.team_away_id.is_none() {
            continue; // skip TBD knockout slots
        }

        let kickoff = r.kickoff_time;
        let mut locked = kickoff.is_some_and(|dt| dt < now);

        // In KO-only leagues group-stage matches are always locked (not tipable).
        if league_config.predict_knockout_only && r.stage == crate::stage::Stage::Group {
            locked = true;
        }
        let finished = r.status == "finished";

        if !locked {
            if let Some(kt) = kickoff {
                next_deadline = Some(match next_deadline {
                    Some(existing) => existing.min(kt),
                    None => kt,
                });
            }
        }

        let own_points = if finished {
            match (
                r.score_home,
                r.score_away,
                r.predicted_home,
                r.predicted_away,
            ) {
                (Some(sh), Some(sa), Some(ph), Some(pa)) => {
                    Some(scoring::calculate_match_points_for_system(
                        league_config.match_scoring_system,
                        r.stage,
                        sh,
                        sa,
                        ph,
                        pa,
                    ))
                }
                _ => None,
            }
        } else {
            None
        };

        let mut other_preds: Vec<UserPrediction> = if locked {
            preds_by_match
                .get(&r.id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|(name, home, away)| {
                    let points = if finished {
                        match (r.score_home, r.score_away) {
                            (Some(sh), Some(sa)) => {
                                Some(scoring::calculate_match_points_for_system(
                                    league_config.match_scoring_system,
                                    r.stage,
                                    sh,
                                    sa,
                                    home,
                                    away,
                                ))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    UserPrediction {
                        name,
                        label: format_prediction_label(
                            league_config.match_scoring_system,
                            r.stage,
                            &r.home_name,
                            &r.away_name,
                            home,
                            away,
                        ),
                        points,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        if finished {
            other_preds.sort_by_key(|b| std::cmp::Reverse(b.points));
        }

        let is_live = r.status == "in_progress";
        let prediction_display = match (r.predicted_home, r.predicted_away) {
            (Some(ph), Some(pa)) => Some(format_prediction_label(
                league_config.match_scoring_system,
                r.stage,
                &r.home_name,
                &r.away_name,
                ph,
                pa,
            )),
            _ => None,
        };
        let mv = MatchView {
            id: r.id,
            stage: r.stage,
            group_letter: r.group_letter.map(|s| s.trim().to_string()),
            home_name: r.home_name,
            away_name: r.away_name,
            home_flag: flag_url(&r.home_flag),
            away_flag: flag_url(&r.away_flag),
            score_home: r.score_home,
            score_away: r.score_away,
            predicted_home: r.predicted_home,
            predicted_away: r.predicted_away,
            prediction_display,
            kickoff_display: format_kickoff(r.kickoff_time),
            locked,
            is_live,
            is_finished: finished,
            own_points,
            max_phase_points: scoring::max_points_for_phase_with_system(
                league_config.match_scoring_system,
                r.stage.to_tournament_phase(),
            ),
            winner_only_mode: league_config.match_scoring_system.is_winner_only(),
            allow_draw_prediction: r.stage == crate::stage::Stage::Group,
            other_preds,
        };

        let target = if locked {
            if finished {
                &mut started_finished
            } else {
                &mut started_in_progress
            }
        } else {
            &mut open_matches
        };
        target.push(mv);
    }

    let group_standings = fetch_group_standings(&state.repos).await;
    let leaderboard = fetch_leaderboard(&state.repos, &state.jerseys, user.league_id, now).await;

    let user_entry = leaderboard.iter().find(|e| e.name == user.name).cloned();
    let user_total_points = user_entry.as_ref().map(|e| e.total_points).unwrap_or(0);

    let user_rank = leaderboard
        .iter()
        .position(|entry| entry.name == user.name)
        .map(|pos| pos + 1)
        .unwrap_or(leaderboard.len() + 1);

    let open_count = open_matches.len();
    let next_deadline_iso = next_deadline.map(|dt| dt.to_rfc3339());
    let started_count = started_in_progress.len() + started_finished.len();

    let special_open = !tournament_locked && special_preds.champion_id.is_none();
    let default_tab = if open_count > 0 {
        "open"
    } else if special_open {
        "special"
    } else {
        "table"
    }
    .to_string();

    let league_name = state
        .repos
        .leagues
        .find_by_id(user.league_id)
        .await
        .ok()
        .flatten()
        .map(|l| l.name)
        .unwrap_or_default();

    let news_items = state.news.get().await;

    let t = crate::handlers::util::t_for(&state, &user.language);
    let lang_code = user.language.clone();

    let badge_ctx_owned = build_badge_context(&state.repos, user.id, user.league_id, now).await;
    let badges_list = badges::compute_all(&badge_ctx_owned.as_ctx(), &t);

    let tipprunden_name = state
        .repos
        .settings
        .get("tipprunden_name")
        .await
        .unwrap_or_default()
        .unwrap_or_else(|| "WM 2026".to_string());

    let template = IndexTemplate {
        user_name: user.name,
        user_total_points,
        user_rank,
        tipprunden_name,
        default_tab,
        started_in_progress,
        started_finished,
        open_matches,
        open_count,
        next_deadline_iso,
        started_count,
        leaderboard,
        group_standings,
        team_options,
        special_preds,
        tournament_locked,
        champ_preds,
        is_admin: user.is_admin,
        can_create_league: user.can_create_league,
        league_id: user.league_id,
        league_name,
        news_items,
        badges: badges_list,
        t,
        lang_code,
        dev_mode: state.dev_mode,
    };
    Ok(render_template(&template)?.into_response())
}

fn format_prediction_label(
    scoring_system: MatchScoringSystem,
    stage: crate::stage::Stage,
    home_name: &str,
    away_name: &str,
    predicted_home: i32,
    predicted_away: i32,
) -> String {
    if scoring_system.is_winner_only() {
        let allow_draw = stage == crate::stage::Stage::Group;
        return scoring::outcome_bet_from_stored_scores(predicted_home, predicted_away, allow_draw)
            .map(|outcome| scoring::winner_only_prediction_label(outcome, home_name, away_name))
            .unwrap_or_else(|| "–".to_string());
    }

    format!("{predicted_home}:{predicted_away}")
}
