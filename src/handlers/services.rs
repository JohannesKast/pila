// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Cross-handler aggregations: leaderboard, group standings, badge context.
//!
//! These functions combine multiple repositories into the higher-level
//! values the index page (and a few sibling handlers) need. Kept SQL-free
//! by routing every read through the repo abstraction, so the same code
//! paths run under Postgres-integration tests and fake-backed unit tests.

use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use crate::badges;
use crate::handlers::util::flag_url;
use crate::jersey::{self, JerseyPreset};
use crate::repo::Repos;
use crate::scoring;
use crate::views::{GroupRow, GroupStandingsTable, LeaderboardEntry};

/// Convenience wrapper — handlers reach for "who actually won the cup?" in
/// several places.
pub async fn fetch_actual_champion(repos: &Repos) -> Option<i32> {
    repos.matches.actual_champion().await.unwrap_or_default()
}

/// Build the read-only context the badge engine consumes. One snapshot per
/// request is shared across every badge implementation — see `badges.rs`.
///
/// All aggregate inputs are filtered by `league_id`: badges compare a user
/// only against league-mates, never across leagues.
pub async fn build_badge_context(
    repos: &Repos,
    user_id: Uuid,
    league_id: Uuid,
    now: DateTime<Utc>,
) -> badges::BadgeContextOwned {
    let league_config = repos
        .leagues
        .get_config(league_id)
        .await
        .unwrap_or_default();
    let finished_predictions: Vec<badges::PredictionRow> = repos
        .predictions
        .list_finished_join(league_id)
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
            scoring_system: league_config.match_scoring_system,
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

    let all_user_ids = repos.users.list_ids(league_id).await.unwrap_or_default();

    let all_special_picks = repos
        .special_predictions
        .list_all_picks(league_id)
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

    let berlin_today = now.with_timezone(&chrono_tz::Europe::Berlin).date_naive();

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

pub async fn fetch_leaderboard(
    repos: &Repos,
    jerseys: &HashMap<String, JerseyPreset>,
    league_id: Uuid,
    now: DateTime<Utc>,
) -> Vec<LeaderboardEntry> {
    let users = repos.users.list_basic(league_id).await.unwrap_or_default();
    let league_config = repos
        .leagues
        .get_config(league_id)
        .await
        .unwrap_or_default();

    let mut user_scores: BTreeMap<String, (i32, i32)> = BTreeMap::new();
    for u in &users {
        user_scores.insert(u.name.clone(), (0, 0));
    }

    let pred_rows = repos
        .predictions
        .list_leaderboard_join(league_id)
        .await
        .unwrap_or_default();

    for r in pred_rows {
        let entry = user_scores.entry(r.user_name.clone()).or_insert((0, 0));
        let started = r.kickoff_time.is_some_and(|dt| dt < now);
        let finished = r.status == "finished";

        if finished {
            if let (Some(sh), Some(sa)) = (r.score_home, r.score_away) {
                entry.0 += scoring::calculate_match_points_for_system(
                    league_config.match_scoring_system,
                    r.stage,
                    sh,
                    sa,
                    r.predicted_home,
                    r.predicted_away,
                );
            }
        } else if started {
            // locked but not finished — full max remains achievable
            entry.1 += scoring::max_potential_points_for_system(
                league_config.match_scoring_system,
                r.stage,
            );
        } else {
            // open — also count max as potential (user already tipped)
            entry.1 += scoring::max_potential_points_for_system(
                league_config.match_scoring_system,
                r.stage,
            );
        }
    }

    let actual_champion = fetch_actual_champion(repos).await;
    let sp_rows = repos
        .special_predictions
        .list_with_user_names(league_id)
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

    let user_jerseys: HashMap<String, String> = users
        .iter()
        .map(|u| (u.name.clone(), u.jersey_preset.clone()))
        .collect();
    let user_ids: HashMap<String, Uuid> = users.iter().map(|u| (u.name.clone(), u.id)).collect();

    let mut leaderboard: Vec<LeaderboardEntry> = user_scores
        .into_iter()
        .map(|(name, (total, potential))| {
            let jersey_preset = user_jerseys
                .get(&name)
                .map(|p| jersey::get(jerseys, p))
                .unwrap_or_else(|| jersey::get(jerseys, "classic"));
            LeaderboardEntry {
                id: user_ids.get(&name).copied().unwrap_or_default(),
                name,
                total_points: total,
                max_potential_points: total + potential,
                jersey_body: jersey_preset.body.clone(),
                jersey_accent: jersey_preset.accent.clone(),
                jersey_pattern: jersey_preset.pattern.clone(),
                // Populated by the dashboard handler from the shared badge context.
                achievements: Vec::new(),
            }
        })
        .collect();
    leaderboard.sort_by_key(|b| std::cmp::Reverse(b.total_points));
    leaderboard
}

pub async fn fetch_group_standings(repos: &Repos) -> Vec<GroupStandingsTable> {
    let rows = repos
        .matches
        .finished_group_rows()
        .await
        .unwrap_or_default();

    // letter → team_id → row
    let mut groups: BTreeMap<String, HashMap<i32, GroupRow>> = BTreeMap::new();

    for r in rows {
        let group = groups.entry(r.group_letter.clone()).or_default();

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
