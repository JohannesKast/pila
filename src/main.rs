use askama::Template;
use axum::{extract::State, routing::get, Router};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use pila::repo::{self, Repos};
use pila::{auth, badges, jersey::JerseyPreset, news, notifier, scoring, stage::Stage, worker, AppState};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Pila Application...");

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    tracing::info!("Running database migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run DB migrations");

    let state = AppState {
        jerseys: pila::jersey::load(),
        news: news::NewsCache::from_env(),
        repos: Repos::from_pool(pool.clone()),
    };

    if let Err(e) = worker::bootstrap_notifications(&pool).await {
        tracing::warn!("Notification bootstrap failed: {:?}", e);
    }

    let notifier = notifier::from_env();
    worker::start_background_worker(pool, notifier).await;

    let app = Router::new()
        .route("/", get(handlers::index))
        .route("/play/me/:token", get(handlers::login_magic_link))
        .route(
            "/setup",
            get(handlers::setup_get).post(handlers::setup_post),
        )
        .route(
            "/predict/:match_id",
            axum::routing::post(handlers::predict_match),
        )
        .route(
            "/predict_special",
            axum::routing::post(handlers::predict_special),
        )
        .route("/leaderboard", get(handlers::leaderboard))
        .route(
            "/profile/jersey-picker",
            get(handlers::jersey_picker_get),
        )
        .route(
            "/profile/jersey-picker/close",
            get(handlers::jersey_picker_close),
        )
        .route(
            "/profile/jersey",
            axum::routing::post(handlers::jersey_post),
        )
        .route(
            "/admin/users",
            axum::routing::post(handlers::admin_create_user),
        )
        .route(
            "/admin/users/:id/delete",
            axum::routing::post(handlers::admin_delete_user),
        )
        .route(
            "/admin/users/:id/promote",
            axum::routing::post(handlers::admin_toggle_admin),
        )
        .route(
            "/admin/users/:id/rename",
            axum::routing::post(handlers::admin_rename_user),
        )
        .route(
            "/admin/users/:id/resend",
            axum::routing::post(handlers::admin_resend_invite),
        )
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".into());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().expect("Invalid PORT");
    tracing::info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

mod handlers {
    use super::*;
    use crate::auth::{AdminUser, AuthenticatedUser, MaybeAuthenticatedUser};
    use axum::{
        extract::{Form, Path},
        http::StatusCode,
        response::{Html, IntoResponse, Redirect, Response},
    };
    use axum_extra::extract::CookieJar;
    use serde::Deserialize;

    fn make_login_cookie(token: String) -> axum_extra::extract::cookie::Cookie<'static> {
        axum_extra::extract::cookie::Cookie::build(("pila_token", token))
            .path("/")
            .http_only(true)
            .secure(true)
            .same_site(axum_extra::extract::cookie::SameSite::Lax)
            .build()
    }

    fn base_url() -> String {
        std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8000".to_string())
    }

    fn build_magic_link(token: &str) -> String {
        format!("{}/play/me/{}", base_url().trim_end_matches('/'), token)
    }

    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }

    pub struct AdminUserView {
        pub id: Uuid,
        pub name: String,
        pub phone_number: Option<String>,
        pub is_admin: bool,
        pub magic_link: String,
        pub is_self: bool,
    }

    async fn fetch_admin_users(repos: &Repos, current_user_id: Uuid) -> Vec<AdminUserView> {
        let rows = repos.users.list_for_admin().await.unwrap_or_default();

        rows.into_iter()
            .map(|r| AdminUserView {
                magic_link: build_magic_link(&r.token),
                is_self: r.id == current_user_id,
                id: r.id,
                name: r.name,
                phone_number: r.phone_number,
                is_admin: r.is_admin,
            })
            .collect()
    }

    // ─── View types ───────────────────────────────────────────────────────────

    pub struct MatchView {
        pub id: i32,
        pub stage: Stage,
        pub stage_label: String,
        pub group_letter: Option<String>,
        pub home_name: String,
        pub away_name: String,
        pub home_flag: String,
        pub away_flag: String,
        pub score_home: Option<i32>,
        pub score_away: Option<i32>,
        pub predicted_home: Option<i32>,
        pub predicted_away: Option<i32>,
        pub kickoff_display: String,
        pub locked: bool,
        pub is_live: bool,
        pub is_finished: bool,
        pub own_points: Option<i32>,
        pub multiplier: i32,
        pub other_preds: Vec<UserPrediction>,
    }

    impl MatchView {
        pub fn predicted_str(&self) -> String {
            match (self.predicted_home, self.predicted_away) {
                (Some(h), Some(a)) => format!("{h}:{a}"),
                _ => "–".to_string(),
            }
        }
        pub fn score_str(&self) -> String {
            match (self.score_home, self.score_away) {
                (Some(h), Some(a)) => format!("{h} : {a}"),
                _ => "– : –".to_string(),
            }
        }
        pub fn has_prediction(&self) -> bool {
            self.predicted_home.is_some() && self.predicted_away.is_some()
        }
    }

    pub struct UserPrediction {
        pub name: String,
        pub home: i32,
        pub away: i32,
        pub points: Option<i32>,
    }

    #[derive(Default)]
    pub struct StageGroups {
        pub groups: Vec<MatchView>,
        pub round_of_32: Vec<MatchView>,
        pub round_of_16: Vec<MatchView>,
        pub quarter_final: Vec<MatchView>,
        pub semi_final: Vec<MatchView>,
        pub third_place: Vec<MatchView>,
        pub final_: Vec<MatchView>,
    }

    impl StageGroups {
        pub fn push(&mut self, m: MatchView) {
            match m.stage {
                Stage::Group => self.groups.push(m),
                Stage::RoundOf32 => self.round_of_32.push(m),
                Stage::RoundOf16 => self.round_of_16.push(m),
                Stage::QuarterFinal => self.quarter_final.push(m),
                Stage::SemiFinal => self.semi_final.push(m),
                Stage::ThirdPlace => self.third_place.push(m),
                Stage::Final => self.final_.push(m),
            }
        }
        pub fn is_empty(&self) -> bool {
            self.groups.is_empty()
                && self.round_of_32.is_empty()
                && self.round_of_16.is_empty()
                && self.quarter_final.is_empty()
                && self.semi_final.is_empty()
                && self.third_place.is_empty()
                && self.final_.is_empty()
        }
        pub fn len(&self) -> usize {
            self.groups.len()
                + self.round_of_32.len()
                + self.round_of_16.len()
                + self.quarter_final.len()
                + self.semi_final.len()
                + self.third_place.len()
                + self.final_.len()
        }
    }

    #[derive(Clone)]
    pub struct TeamView {
        pub id: i32,
        pub name: String,
    }

    pub struct SpecialPredictionsView {
        pub champion_id: Option<i32>,
    }

    impl SpecialPredictionsView {
        pub fn is_champion(&self, team_id: &i32) -> bool {
            self.champion_id == Some(*team_id)
        }
    }

    pub struct ChampPrediction {
        pub name: String,
        pub team_name: String,
        pub team_flag: String,
        pub points: Option<i32>,
    }

    pub struct GroupRow {
        pub team_name: String,
        pub flag: String,
        pub played: i32,
        pub wins: i32,
        pub draws: i32,
        pub losses: i32,
        pub goals_for: i32,
        pub goals_against: i32,
        pub goal_diff: i32,
        pub points: i32,
    }

    pub struct GroupStandingsTable {
        pub letter: String,
        pub rows: Vec<GroupRow>,
    }

    #[derive(Clone)]
    pub struct LeaderboardEntry {
        pub name: String,
        pub total_points: i32,
        pub max_potential_points: i32,
        pub jersey_body: String,
        pub jersey_accent: String,
        pub jersey_pattern: String,
    }

    // ─── Helpers ──────────────────────────────────────────────────────────────

    fn flag_url(code: &Option<String>) -> String {
        match code {
            Some(c) if !c.is_empty() => format!("https://flagcdn.com/w40/{c}.png"),
            _ => String::new(),
        }
    }

    fn format_kickoff(dt: Option<chrono::DateTime<chrono::Utc>>) -> String {
        match dt {
            Some(d) => d
                .with_timezone(&chrono_tz::Europe::Berlin)
                .format("%d.%m.%Y %H:%M")
                .to_string(),
            None => "TBD".to_string(),
        }
    }

    async fn fetch_actual_champion(repos: &Repos) -> Option<i32> {
        repos.matches.actual_champion().await.unwrap_or_default()
    }

    async fn build_badge_context(
        repos: &Repos,
        user_id: Uuid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> badges::BadgeContextOwned {
        let finished_predictions: Vec<badges::PredictionRow> = repos
            .predictions
            .list_finished_join()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|r| badges::PredictionRow {
                user_id: r.user_id,
                match_id: r.match_id,
                stage: r.stage,
                kickoff: r.kickoff,
                score_h: r.score_home,
                score_a: r.score_away,
                pred_h: r.predicted_home,
                pred_a: r.predicted_away,
            })
            .collect();

        let started_matches_total = repos
            .matches
            .started_with_both_teams_count(now)
            .await
            .unwrap_or(0) as i32;

        let user_started_tips = repos
            .predictions
            .count_user_started(user_id, now)
            .await
            .unwrap_or(0) as i32;

        let all_user_ids = repos.users.list_ids().await.unwrap_or_default();

        let all_special_picks = repos
            .special_predictions
            .list_all_picks()
            .await
            .unwrap_or_default();

        let actual_champion_id = fetch_actual_champion(repos).await;

        let user_champion = repos
            .special_predictions
            .user_champion_view(user_id)
            .await
            .unwrap_or_default()
            .map(|c| badges::ChampionView {
                team_name: c.team_name,
                flag_url: flag_url(&c.flag_code),
            });

        let berlin_today = now
            .with_timezone(&chrono_tz::Europe::Berlin)
            .date_naive();

        badges::BadgeContextOwned {
            user_id,
            now,
            berlin_today,
            finished_predictions,
            all_user_ids,
            started_matches_total,
            user_started_tips,
            actual_champion_id,
            all_special_picks,
            user_champion,
        }
    }

    async fn fetch_leaderboard(
        repos: &Repos,
        jerseys: &std::collections::HashMap<String, JerseyPreset>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Vec<LeaderboardEntry> {
        let users = repos.users.list_basic().await.unwrap_or_default();

        let mut user_scores: std::collections::BTreeMap<String, (i32, i32)> =
            std::collections::BTreeMap::new();
        for u in &users {
            user_scores.insert(u.name.clone(), (0, 0));
        }

        let pred_rows = repos
            .predictions
            .list_leaderboard_join()
            .await
            .unwrap_or_default();

        for r in pred_rows {
            let entry = user_scores.entry(r.user_name.clone()).or_insert((0, 0));
            let started = r.kickoff_time.is_some_and(|dt| dt < now);
            let finished = r.status == "finished";

            if finished {
                if let (Some(sh), Some(sa)) = (r.score_home, r.score_away) {
                    entry.0 += scoring::calculate_match_points(
                        r.stage,
                        sh,
                        sa,
                        r.predicted_home,
                        r.predicted_away,
                    );
                }
            } else if started {
                // locked but not finished — full max remains achievable
                entry.1 += scoring::max_potential_points(r.stage);
            } else {
                // open — also count max as potential (user already tipped)
                entry.1 += scoring::max_potential_points(r.stage);
            }
        }

        let actual_champion = fetch_actual_champion(repos).await;
        let sp_rows = repos
            .special_predictions
            .list_with_user_names()
            .await
            .unwrap_or_default();

        for sp in sp_rows {
            let entry = user_scores.entry(sp.user_name).or_insert((0, 0));
            if let Some(cid) = sp.champion_id {
                if actual_champion.is_some() {
                    entry.0 += scoring::champion_points(Some(cid), actual_champion);
                } else {
                    entry.1 += 10;
                }
            }
        }

        let user_jerseys: std::collections::HashMap<String, String> = users
            .iter()
            .map(|u| (u.name.clone(), u.jersey_preset.clone()))
            .collect();

        let mut leaderboard: Vec<LeaderboardEntry> = user_scores
            .into_iter()
            .map(|(name, (total, potential))| {
                let jersey_preset = user_jerseys
                    .get(&name)
                    .and_then(|p| jerseys.get(p))
                    .unwrap_or_else(|| jerseys.get("classic").unwrap());
                LeaderboardEntry {
                    name,
                    total_points: total,
                    max_potential_points: total + potential,
                    jersey_body: jersey_preset.body.clone(),
                    jersey_accent: jersey_preset.accent.clone(),
                    jersey_pattern: jersey_preset.pattern.clone(),
                }
            })
            .collect();
        leaderboard.sort_by(|a, b| b.total_points.cmp(&a.total_points));
        leaderboard
    }

    async fn fetch_group_standings(repos: &Repos) -> Vec<GroupStandingsTable> {
        let rows = repos
            .matches
            .finished_group_rows()
            .await
            .unwrap_or_default();

        // letter → team_id → row
        let mut groups: std::collections::BTreeMap<String, std::collections::HashMap<i32, GroupRow>> =
            std::collections::BTreeMap::new();

        for r in rows {
            let letter = r.group_letter.clone();
            let group = groups.entry(letter).or_default();

            let home = group.entry(r.home_id).or_insert_with(|| GroupRow {
                team_name: r.home_name.clone(),
                flag: flag_url(&r.home_flag),
                played: 0,
                wins: 0,
                draws: 0,
                losses: 0,
                goals_for: 0,
                goals_against: 0,
                goal_diff: 0,
                points: 0,
            });
            home.played += 1;
            home.goals_for += r.score_home;
            home.goals_against += r.score_away;
            if r.score_home > r.score_away {
                home.wins += 1;
                home.points += 3;
            } else if r.score_home == r.score_away {
                home.draws += 1;
                home.points += 1;
            } else {
                home.losses += 1;
            }
            home.goal_diff = home.goals_for - home.goals_against;

            let away = group.entry(r.away_id).or_insert_with(|| GroupRow {
                team_name: r.away_name.clone(),
                flag: flag_url(&r.away_flag),
                played: 0,
                wins: 0,
                draws: 0,
                losses: 0,
                goals_for: 0,
                goals_against: 0,
                goal_diff: 0,
                points: 0,
            });
            away.played += 1;
            away.goals_for += r.score_away;
            away.goals_against += r.score_home;
            if r.score_away > r.score_home {
                away.wins += 1;
                away.points += 3;
            } else if r.score_away == r.score_home {
                away.draws += 1;
                away.points += 1;
            } else {
                away.losses += 1;
            }
            away.goal_diff = away.goals_for - away.goals_against;
        }

        groups
            .into_iter()
            .map(|(letter, teams)| {
                let mut rows: Vec<GroupRow> = teams.into_values().collect();
                rows.sort_by(|a, b| {
                    b.points
                        .cmp(&a.points)
                        .then(b.goal_diff.cmp(&a.goal_diff))
                        .then(b.goals_for.cmp(&a.goals_for))
                        .then(a.team_name.cmp(&b.team_name))
                });
                GroupStandingsTable { letter, rows }
            })
            .collect()
    }

    // ─── Templates ────────────────────────────────────────────────────────────

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
        admin_users: Vec<AdminUserView>,
        signal_enabled: bool,
        news_items: Vec<pila::news::NewsItem>,
        badges: Vec<badges::BadgeView>,
    }

    #[derive(Template)]
    #[template(path = "setup.html")]
    struct SetupTemplate {}

    #[derive(Template)]
    #[template(path = "admin_row.html")]
    struct AdminRowTemplate {
        u: AdminUserView,
        signal_enabled: bool,
    }

    #[derive(Template)]
    #[template(path = "leaderboard.html")]
    struct LeaderboardTemplate {
        entries: Vec<LeaderboardEntry>,
    }

    pub struct JerseyOption {
        pub key: String,
        pub preset: JerseyPreset,
    }

    #[derive(Template)]
    #[template(path = "jersey_picker.html")]
    struct JerseyPickerTemplate {
        options: Vec<JerseyOption>,
        current: String,
    }

    #[derive(Template)]
    #[template(path = "leaderboard_entry.html")]
    struct LeaderboardEntryTemplate {
        entry: LeaderboardEntry,
        rank: usize,
    }

    // ─── Handlers ─────────────────────────────────────────────────────────────

    pub async fn index(
        State(state): State<AppState>,
        MaybeAuthenticatedUser(maybe_user): MaybeAuthenticatedUser,
    ) -> Result<Response, (StatusCode, &'static str)> {
        let user = match maybe_user {
            Some(u) => u,
            None => {
                let count = state
                    .repos
                    .users
                    .count()
                    .await
                    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
                if count == 0 {
                    return Ok(Redirect::to("/setup").into_response());
                }
                return Err((
                    StatusCode::UNAUTHORIZED,
                    "Nicht authentifiziert. Bitte nutze deinen Magic Link (z.B. /play/me/mein-token).",
                ));
            }
        };
        let now = chrono::Utc::now();

        let rows = state
            .repos
            .matches
            .list_for_index(user.id)
            .await
            .unwrap_or_default();

        let other_preds_rows = state
            .repos
            .predictions
            .list_other_users_locked(user.id, now)
            .await
            .unwrap_or_default();

        let mut preds_by_match: std::collections::HashMap<i32, Vec<(String, i32, i32)>> =
            std::collections::HashMap::new();
        for p in other_preds_rows {
            preds_by_match
                .entry(p.match_id)
                .or_default()
                .push((p.user_name, p.predicted_home, p.predicted_away));
        }

        let first_kickoff = state
            .repos
            .matches
            .first_kickoff()
            .await
            .unwrap_or_default();
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
                .list_with_user_names()
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
            let locked = kickoff.is_some_and(|dt| dt < now);
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
                        Some(scoring::calculate_match_points(r.stage, sh, sa, ph, pa))
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
                                (Some(sh), Some(sa)) => Some(scoring::calculate_match_points(
                                    r.stage, sh, sa, home, away,
                                )),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        UserPrediction {
                            name,
                            home,
                            away,
                            points,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if finished {
                other_preds.sort_by(|a, b| b.points.cmp(&a.points));
            }

            let is_live = r.status == "in_progress";
            let mv = MatchView {
                id: r.id,
                stage: r.stage,
                stage_label: r.stage.label_de().to_string(),
                group_letter: r.group_letter.map(|s| s.trim().to_string()),
                home_name: r.home_name,
                away_name: r.away_name,
                home_flag: flag_url(&r.home_flag),
                away_flag: flag_url(&r.away_flag),
                score_home: r.score_home,
                score_away: r.score_away,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
                kickoff_display: format_kickoff(r.kickoff_time),
                locked,
                is_live,
                is_finished: finished,
                own_points,
                multiplier: r.stage.multiplier(),
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
        let leaderboard = fetch_leaderboard(&state.repos, &state.jerseys, now).await;

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

        let admin_users = if user.is_admin {
            fetch_admin_users(&state.repos, user.id).await
        } else {
            Vec::new()
        };

        let news_items = state.news.get().await;

        let badge_ctx_owned = build_badge_context(&state.repos, user.id, now).await;
        let badges_list = badges::compute_all(&badge_ctx_owned.as_ctx());

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
            admin_users,
            signal_enabled: notifier::signal_configured(),
            news_items,
            badges: badges_list,
        };
        Ok(Html(template.render().unwrap()).into_response())
    }

    pub async fn login_magic_link(
        State(state): State<AppState>,
        Path(token): Path<String>,
        jar: CookieJar,
    ) -> Result<(CookieJar, Redirect), (StatusCode, &'static str)> {
        let exists = state
            .repos
            .users
            .find_by_token(&token)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
            .is_some();

        if exists {
            let cookie = axum_extra::extract::cookie::Cookie::build(("pila_token", token))
                .path("/")
                .http_only(true)
                .secure(true)
                .same_site(axum_extra::extract::cookie::SameSite::Lax)
                .build();
            let updated_jar = jar.add(cookie);
            Ok((updated_jar, Redirect::to("/")))
        } else {
            Err((StatusCode::UNAUTHORIZED, "Ungültiger oder abgelaufener Link."))
        }
    }

    #[derive(Deserialize)]
    pub struct PredictionForm {
        score_home: i32,
        score_away: i32,
    }

    pub async fn predict_match(
        State(state): State<AppState>,
        user: AuthenticatedUser,
        Path(match_id): Path<i32>,
        Form(form): Form<PredictionForm>,
    ) -> Result<Html<String>, (StatusCode, &'static str)> {
        if !(0..=20).contains(&form.score_home) || !(0..=20).contains(&form.score_away) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Ungültiger Tipp: Werte müssen zwischen 0 und 20 liegen.",
            ));
        }

        let m = state
            .repos
            .matches
            .find_lock_info(match_id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
            .ok_or((StatusCode::NOT_FOUND, "Match not found"))?;

        let now = chrono::Utc::now();

        if m.team_home_id.is_none() || m.team_away_id.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Begegnung steht noch nicht fest. Tipp nicht möglich.",
            ));
        }

        if let Some(start_time) = m.kickoff_time {
            if start_time < now {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Das Spiel hat bereits begonnen. Tipps sind gesperrt.",
                ));
            }
        }

        state
            .repos
            .predictions
            .upsert(user.id, match_id, form.score_home, form.score_away)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

        let html = format!(
            r##"<form id="tip-form-{id}" hx-post="/predict/{id}" hx-swap="outerHTML" hx-target="#tip-form-{id}" style="display:flex; align-items:center; gap:6px;">
  <input class="pl-num" type="number" name="score_home" min="0" max="20" value="{h}" required style="width:42px; height:42px; text-align:center; background:#06090a; border:1.5px solid var(--pl-green); border-radius:8px; color:var(--pl-fg); font-size:18px; outline:none; box-shadow:0 0 12px rgba(116,255,140,.3);">
  <span class="pl-mono" style="color:var(--pl-mute);">:</span>
  <input class="pl-num" type="number" name="score_away" min="0" max="20" value="{a}" required style="width:42px; height:42px; text-align:center; background:#06090a; border:1.5px solid var(--pl-green); border-radius:8px; color:var(--pl-fg); font-size:18px; outline:none; box-shadow:0 0 12px rgba(116,255,140,.3);">
  <button type="submit" class="pl-btn pl-btn--primary" style="height:42px; padding:0 12px; font-size:11px;">OK</button>
</form>"##,
            id = match_id, h = form.score_home, a = form.score_away
        );
        Ok(Html(html))
    }

    fn deserialize_optional_int<'de, D>(de: D) -> Result<Option<i32>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Option::<String>::deserialize(de)?;
        match s.as_deref() {
            None | Some("") => Ok(None),
            Some(s) => s.parse().map(Some).map_err(serde::de::Error::custom),
        }
    }

    #[derive(Deserialize)]
    pub struct SpecialPredictionForm {
        #[serde(default, deserialize_with = "deserialize_optional_int")]
        champion_id: Option<i32>,
    }

    pub async fn predict_special(
        State(state): State<AppState>,
        user: AuthenticatedUser,
        Form(form): Form<SpecialPredictionForm>,
    ) -> Result<Redirect, (StatusCode, &'static str)> {
        let now = chrono::Utc::now();

        let first_kickoff = state
            .repos
            .matches
            .first_kickoff()
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        if first_kickoff.is_some_and(|dt| dt < now) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Turnier hat begonnen. Weltmeister-Tipp ist gesperrt.",
            ));
        }

        if let Some(cid) = form.champion_id {
            let exists = state
                .repos
                .teams
                .exists_real(cid)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
            if !exists {
                return Err((StatusCode::BAD_REQUEST, "Unbekanntes Team."));
            }
        }

        state
            .repos
            .special_predictions
            .upsert(user.id, form.champion_id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        Ok(Redirect::to("/"))
    }

    pub async fn leaderboard(
        State(state): State<AppState>,
        _user: AuthenticatedUser,
    ) -> Html<String> {
        let now = chrono::Utc::now();
        let entries = fetch_leaderboard(&state.repos, &state.jerseys, now).await;
        let template = LeaderboardTemplate { entries };
        Html(template.render().unwrap())
    }

    // ─── Setup (first-run admin creation) ────────────────────────────────────

    async fn user_count(repos: &Repos) -> Result<i64, (StatusCode, &'static str)> {
        repos
            .users
            .count()
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))
    }

    pub async fn setup_get(
        State(state): State<AppState>,
    ) -> Result<Response, (StatusCode, &'static str)> {
        if user_count(&state.repos).await? > 0 {
            return Ok(Redirect::to("/").into_response());
        }
        let template = SetupTemplate {};
        Ok(Html(template.render().unwrap()).into_response())
    }

    #[derive(Deserialize)]
    pub struct SetupForm {
        name: String,
        #[serde(default)]
        phone_number: String,
    }

    pub async fn setup_post(
        State(state): State<AppState>,
        jar: CookieJar,
        Form(form): Form<SetupForm>,
    ) -> Result<(CookieJar, Redirect), (StatusCode, &'static str)> {
        if user_count(&state.repos).await? > 0 {
            return Err((StatusCode::FORBIDDEN, "Setup bereits abgeschlossen."));
        }
        let name = form.name.trim();
        if name.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Name darf nicht leer sein."));
        }
        let phone = form.phone_number.trim();
        let phone_opt: Option<&str> = if phone.is_empty() { None } else { Some(phone) };

        let id = Uuid::new_v4();
        let token = Uuid::new_v4().to_string();

        state
            .repos
            .users
            .create(repo::user::NewUser {
                id,
                name,
                token: &token,
                is_admin: true,
                phone_number: phone_opt,
            })
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        if let Some(p) = phone_opt {
            if notifier::signal_configured() {
                let link = build_magic_link(&token);
                if let Err(e) = notifier::send_invite_via_signal(p, name, &link).await {
                    tracing::warn!("Setup: Signal-Einladung an {p} fehlgeschlagen: {e}");
                }
            }
        }

        let updated_jar = jar.add(make_login_cookie(token));
        Ok((updated_jar, Redirect::to("/")))
    }

    // ─── Admin handlers ──────────────────────────────────────────────────────

    fn render_admin_row(u: AdminUserView, signal_enabled: bool) -> Html<String> {
        let tpl = AdminRowTemplate { u, signal_enabled };
        Html(tpl.render().unwrap())
    }

    #[derive(Deserialize)]
    pub struct AdminCreateForm {
        name: String,
        #[serde(default)]
        phone_number: String,
    }

    pub async fn admin_create_user(
        State(state): State<AppState>,
        AdminUser(_admin): AdminUser,
        Form(form): Form<AdminCreateForm>,
    ) -> Result<Html<String>, (StatusCode, &'static str)> {
        let name = form.name.trim();
        if name.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Name darf nicht leer sein."));
        }
        let phone = form.phone_number.trim();
        let phone_opt: Option<&str> = if phone.is_empty() { None } else { Some(phone) };

        let id = Uuid::new_v4();
        let token = Uuid::new_v4().to_string();

        state
            .repos
            .users
            .create(repo::user::NewUser {
                id,
                name,
                token: &token,
                is_admin: false,
                phone_number: phone_opt,
            })
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        let signal_enabled = notifier::signal_configured();
        let link = build_magic_link(&token);
        if let Some(p) = phone_opt {
            if signal_enabled {
                if let Err(e) = notifier::send_invite_via_signal(p, name, &link).await {
                    tracing::warn!("Admin: Signal-Einladung an {p} fehlgeschlagen: {e}");
                }
            }
        }

        let view = AdminUserView {
            id,
            name: name.to_string(),
            phone_number: phone_opt.map(|s| s.to_string()),
            is_admin: false,
            magic_link: link,
            is_self: false,
        };
        Ok(render_admin_row(view, signal_enabled))
    }

    pub async fn admin_delete_user(
        State(state): State<AppState>,
        AdminUser(admin): AdminUser,
        Path(id): Path<Uuid>,
    ) -> Result<Html<String>, (StatusCode, &'static str)> {
        if id == admin.id {
            return Err((
                StatusCode::BAD_REQUEST,
                "Du kannst dich nicht selbst löschen.",
            ));
        }
        state
            .repos
            .users
            .delete(id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
        Ok(Html(String::new()))
    }

    pub async fn admin_toggle_admin(
        State(state): State<AppState>,
        AdminUser(admin): AdminUser,
        Path(id): Path<Uuid>,
    ) -> Result<Html<String>, (StatusCode, &'static str)> {
        let target = state
            .repos
            .users
            .find_full_by_id(id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
            .ok_or((StatusCode::NOT_FOUND, "User nicht gefunden."))?;

        let new_admin = !target.is_admin;
        if !new_admin {
            let admin_count = state
                .repos
                .users
                .count_admins()
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
            if admin_count <= 1 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Mindestens ein Admin muss bestehen bleiben.",
                ));
            }
            if id == admin.id {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Du kannst dir nicht selbst die Adminrechte entziehen.",
                ));
            }
        }

        state
            .repos
            .users
            .set_admin(id, new_admin)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        let view = AdminUserView {
            magic_link: build_magic_link(&target.token),
            is_self: id == admin.id,
            id,
            name: target.name,
            phone_number: target.phone_number,
            is_admin: new_admin,
        };
        Ok(render_admin_row(view, notifier::signal_configured()))
    }

    #[derive(Deserialize)]
    pub struct AdminRenameForm {
        name: String,
    }

    pub async fn admin_rename_user(
        State(state): State<AppState>,
        AdminUser(admin): AdminUser,
        Path(id): Path<Uuid>,
        Form(form): Form<AdminRenameForm>,
    ) -> Result<Html<String>, (StatusCode, &'static str)> {
        let name = form.name.trim();
        if name.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Name darf nicht leer sein."));
        }
        state
            .repos
            .users
            .rename(id, name)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        let target = state
            .repos
            .users
            .find_full_by_id(id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
            .ok_or((StatusCode::NOT_FOUND, "User nicht gefunden."))?;

        let view = AdminUserView {
            magic_link: build_magic_link(&target.token),
            is_self: id == admin.id,
            id,
            name: target.name,
            phone_number: target.phone_number,
            is_admin: target.is_admin,
        };
        Ok(render_admin_row(view, notifier::signal_configured()))
    }

    pub async fn admin_resend_invite(
        State(state): State<AppState>,
        AdminUser(_admin): AdminUser,
        Path(id): Path<Uuid>,
    ) -> Html<String> {
        let row = state.repos.users.find_full_by_id(id).await.ok().flatten();
        let Some(row) = row else {
            return Html(
                r#"<span class="text-red-400 text-xs">User nicht gefunden</span>"#.to_string(),
            );
        };
        let Some(phone) = row.phone_number.as_deref() else {
            return Html(
                r#"<span class="text-amber-400 text-xs">Keine Telefonnummer hinterlegt</span>"#
                    .to_string(),
            );
        };
        if !notifier::signal_configured() {
            return Html(
                r#"<span class="text-amber-400 text-xs">Signal nicht konfiguriert</span>"#
                    .to_string(),
            );
        }
        let link = build_magic_link(&row.token);
        match notifier::send_invite_via_signal(phone, &row.name, &link).await {
            Ok(_) => Html(
                r#"<span class="text-emerald-400 text-xs">✓ gesendet</span>"#.to_string(),
            ),
            Err(e) => Html(format!(
                r#"<span class="text-red-400 text-xs">✗ Fehler: {}</span>"#,
                html_escape(&e.to_string())
            )),
        }
    }

    // ─── Jersey picker ────────────────────────────────────────────────────────

    pub async fn jersey_picker_get(
        State(state): State<AppState>,
        user: AuthenticatedUser,
    ) -> Html<String> {
        let mut options: Vec<JerseyOption> = state
            .jerseys
            .iter()
            .map(|(k, v)| JerseyOption {
                key: k.clone(),
                preset: v.clone(),
            })
            .collect();
        options.sort_by(|a, b| {
            let pila_a = a.preset.group == "Pila";
            let pila_b = b.preset.group == "Pila";
            pila_b
                .cmp(&pila_a)
                .then_with(|| a.preset.group.cmp(&b.preset.group))
                .then_with(|| a.preset.name.cmp(&b.preset.name))
        });

        let template = JerseyPickerTemplate {
            options,
            current: user.jersey_preset,
        };
        Html(template.render().unwrap())
    }

    pub async fn jersey_picker_close() -> Html<&'static str> {
        Html("")
    }

    #[derive(Deserialize)]
    pub struct JerseyPostQuery {
        preset: String,
    }

    pub async fn jersey_post(
        State(state): State<AppState>,
        user: AuthenticatedUser,
        axum::extract::Query(q): axum::extract::Query<JerseyPostQuery>,
    ) -> Result<Html<String>, (StatusCode, &'static str)> {
        if !state.jerseys.contains_key(&q.preset) {
            return Err((StatusCode::BAD_REQUEST, "Unbekanntes Trikot."));
        }
        state
            .repos
            .users
            .set_jersey(user.id, &q.preset)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

        let leaderboard =
            fetch_leaderboard(&state.repos, &state.jerseys, chrono::Utc::now()).await;
        let user_rank = leaderboard
            .iter()
            .position(|e| e.name == user.name)
            .map(|p| p + 1)
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "User not in leaderboard"))?;
        let user_entry = leaderboard[user_rank - 1].clone();

        let entry_template = LeaderboardEntryTemplate {
            entry: user_entry,
            rank: user_rank,
        };
        let entry_html = entry_template.render().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Template render error",
            )
        })?;

        let oob_html = format!(
            r#"<div style="display:flex; align-items:center; gap:10px; padding:12px 14px; border-bottom:1px solid var(--pl-line); background:rgba(255,230,0,.04)" hx-swap-oob="innerHTML" id="leaderboard-entry-{}">{}</div>"#,
            html_escape(&user.name.to_lowercase()),
            entry_html
        );

        Ok(Html(oob_html))
    }
}
