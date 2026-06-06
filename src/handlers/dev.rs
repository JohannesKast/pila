// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Dev/testing routes for simulating tournament progression.
//!
//! **Only active when `PILA_DEV_MODE=true`.** These routes allow:
//! - Setting mock time to simulate tournament progression
//! - Generating random tips for users
//! - Setting random match results
//! - Simulating "next matchday" (time jump + results)
//! - Switching between users for testing
//!
//! All routes check `state.dev_mode` and return 404 if not enabled.

use crate::repo::fixture::IndexMatchRow;
use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect},
};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::handlers::util::render_template;
use crate::time;
use crate::AppState;

// Empty form for POST handlers that don't need data
#[derive(Deserialize)]
pub struct EmptyForm {}

/// Empty 200 response with the `HX-Refresh: true` header. HTMX reacts by
/// triggering a full `window.location.reload()` in the browser, so the
/// background index/leaderboard reflects the mutated DB state immediately.
fn htmx_refresh() -> impl IntoResponse {
    ([("HX-Refresh", "true")], "")
}

/// Random goals per team, weighted to resemble real soccer scoring (rough
/// Poisson(λ≈1.4) approximation). A uniform 0..=5 made 0:0 and other
/// low-spread draws far too common.
fn random_goals(rng: &mut impl Rng) -> i32 {
    let r: f64 = rng.random();
    if r < 0.24 {
        0
    } else if r < 0.58 {
        1
    } else if r < 0.82 {
        2
    } else if r < 0.94 {
        3
    } else if r < 0.98 {
        4
    } else {
        5
    }
}

fn random_result(rng: &mut impl Rng) -> (i32, i32) {
    (random_goals(rng), random_goals(rng))
}

/// Returns all matches that belong to the same Berlin calendar day as the
/// earliest unstarted, fully-known fixture. Returns `None` when no future
/// tippable match exists.
fn find_next_unstarted_matchday(
    matches: &[IndexMatchRow],
    now: DateTime<Utc>,
) -> Option<Vec<IndexMatchRow>> {
    use chrono_tz::Europe::Berlin;

    let next = matches
        .iter()
        .filter(|m| m.team_home_id.is_some() && m.team_away_id.is_some())
        .filter(|m| m.kickoff_time.is_some_and(|k| k > now))
        .min_by_key(|m| m.kickoff_time)?;

    let matchday = next
        .kickoff_time
        .expect("filter above guarantees kickoff_time is Some")
        .with_timezone(&Berlin)
        .date_naive();

    let result = matches
        .iter()
        .filter(|m| m.team_home_id.is_some() && m.team_away_id.is_some())
        .filter(|m| {
            m.kickoff_time
                .is_some_and(|k| k > now && k.with_timezone(&Berlin).date_naive() == matchday)
        })
        .cloned()
        .collect();

    Some(result)
}

// ─── Templates ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "dev/panel.html")]
struct DevPanelTemplate {
    current_time: Option<String>,
    users: Vec<UserSummary>,
    current_user_id: Uuid,
    match_count: usize,
    unstarted_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub name: String,
    pub is_admin: bool,
}

// ─── Route: GET /dev ──────────────────────────────────────────────────────────

pub async fn dev_panel(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Html<String>, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }
    render_panel(&state, &user).await
}

/// Renders the dev panel fragment. Used by GET /dev and as the HTMX response
/// of every mutation handler so the panel stays open after a form submit.
async fn render_panel(
    state: &AppState,
    user: &AuthenticatedUser,
) -> Result<Html<String>, (StatusCode, String)> {
    let current_time = state
        .mock_now
        .read()
        .ok()
        .and_then(|guard| guard.map(|t| t.format("%Y-%m-%dT%H:%M").to_string()));

    let users_raw = state
        .repos
        .users
        .list_basic(user.league_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;

    let users: Vec<UserSummary> = users_raw
        .into_iter()
        .map(|u| UserSummary {
            id: u.id,
            name: u.name,
            is_admin: false,
        })
        .collect();

    let matches = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;

    let now = time::now(&state.mock_now);
    let unstarted_count = matches
        .iter()
        .filter(|m| m.kickoff_time.is_some_and(|k| k > now))
        .count();

    let template = DevPanelTemplate {
        current_time,
        users,
        current_user_id: user.id,
        match_count: matches.len(),
        unstarted_count,
    };

    render_template(&template).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Template error".to_string(),
        )
    })
}

// ─── Route: POST /dev/time ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SetTimeForm {
    pub time: String, // ISO datetime: 2026-06-12T14:00
}

pub async fn dev_set_time(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
    Form(form): Form<SetTimeForm>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    // The HTML <input type="datetime-local"> field emits values like
    // `2026-06-12T18:00` (no seconds, no timezone) — not RFC3339. Parse as
    // a naive local-wallclock value, then treat it as UTC.
    let naive = NaiveDateTime::parse_from_str(&form.time, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(&form.time, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid datetime format (use YYYY-MM-DDTHH:MM)".to_string(),
            )
        })?;
    let parsed = Utc.from_utc_datetime(&naive);

    time::set_mock_time(&state.mock_now, parsed);
    tracing::info!("🕐 Mock time set to: {}", parsed);

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/time/reset ─────────────────────────────────────────────

pub async fn dev_reset_time(
    State(state): State<AppState>,
    _user: AuthenticatedUser,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    time::clear_mock_time(&state.mock_now);
    tracing::info!("🕐 Mock time reset to real time");

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/tips/random ────────────────────────────────────────────

pub async fn dev_random_tips(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    _form: Form<EmptyForm>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    let matches = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;

    let now = time::now(&state.mock_now);
    let mut rng = StdRng::from_os_rng();

    // Tip all matches that haven't started yet and don't have a tip yet
    for m in matches {
        // Skip if already started
        if m.kickoff_time.is_some_and(|k| k <= now) {
            continue;
        }
        // Skip if already tipped
        if m.predicted_home.is_some() {
            continue;
        }
        // Skip if teams not set
        if m.team_home_id.is_none() || m.team_away_id.is_none() {
            continue;
        }

        let home = random_goals(&mut rng);
        let away = random_goals(&mut rng);

        state
            .repos
            .predictions
            .upsert(user.id, m.id, home, away)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            })?;
    }

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/tips/all-users ────────────────────────────────────────

pub async fn dev_random_tips_all_users(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    _form: Form<EmptyForm>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    let user_ids = state
        .repos
        .users
        .list_ids(user.league_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;

    let now = time::now(&state.mock_now);
    let mut rng = StdRng::from_os_rng();

    for uid in user_ids {
        let matches = state.repos.matches.list_for_index(uid).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;

        for m in matches {
            if m.kickoff_time.is_some_and(|k| k <= now) {
                continue;
            }
            if m.team_home_id.is_none() || m.team_away_id.is_none() {
                continue;
            }

            let home = random_goals(&mut rng);
            let away = random_goals(&mut rng);

            state
                .repos
                .predictions
                .upsert(uid, m.id, home, away)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Database error".to_string(),
                    )
                })?;
        }
    }

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/results/random ────────────────────────────────────────

pub async fn dev_random_results(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    _form: Form<EmptyForm>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    let matches = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;

    let now = time::now(&state.mock_now);
    let mut rng = StdRng::from_os_rng();

    for m in matches {
        if m.kickoff_time.is_none() || m.kickoff_time.is_some_and(|k| k > now) {
            continue;
        }
        if m.team_home_id.is_none() || m.team_away_id.is_none() {
            continue;
        }

        // Always generate fresh scores. Status forced to "finished" so the
        // index handler buckets the match correctly even after an ESPN re-sync
        // reverted the status field to "scheduled".
        let home = random_goals(&mut rng);
        let away = random_goals(&mut rng);

        state
            .repos
            .matches
            .update_result(m.id, Some(home), Some(away), "finished")
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            })?;
    }

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/simulate/next-matchday ─────────────────────────────────

pub async fn dev_simulate_next_matchday(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    _form: Form<EmptyForm>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    let db_err = || {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database error".to_string(),
        )
    };
    let matches = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .map_err(|_| db_err())?;
    let now = time::now(&state.mock_now);

    let matchday = find_next_unstarted_matchday(&matches, now).ok_or_else(|| {
        tracing::warn!(
            "dev_simulate_next_matchday: no future tippable match (mock_now={}, matches={})",
            now,
            matches.len()
        );
        (
            StatusCode::BAD_REQUEST,
            "No future match found. Matches table empty or mock time past the final?".to_string(),
        )
    })?;

    let user_ids = state
        .repos
        .users
        .list_ids(user.league_id)
        .await
        .map_err(|_| db_err())?;
    let mut rng = StdRng::from_os_rng();

    for m in &matchday {
        for uid in &user_ids {
            if *uid == user.id && m.predicted_home.is_some() {
                continue;
            }
            let (home, away) = random_result(&mut rng);
            state
                .repos
                .predictions
                .upsert(*uid, m.id, home, away)
                .await
                .map_err(|_| db_err())?;
        }
    }

    let new_time = matchday
        .iter()
        .filter_map(|m| m.kickoff_time)
        .max()
        .expect("find_next_unstarted_matchday guarantees kickoff_time is Some")
        + chrono::Duration::hours(2);
    time::set_mock_time(&state.mock_now, new_time);

    tracing::info!(
        "🎮 Updating {} matchday matches with results",
        matchday.len()
    );
    for m in &matchday {
        let (home, away) = random_result(&mut rng);
        tracing::info!(
            "🎮 Setting result: match {} ({} vs {}) → {}:{}",
            m.id,
            m.home_name,
            m.away_name,
            home,
            away
        );
        state
            .repos
            .matches
            .update_result(m.id, Some(home), Some(away), "finished")
            .await
            .map_err(|_| db_err())?;
    }

    tracing::info!(
        "🎮 Simulated matchday ({} matches, {} users), mock_now: {}",
        matchday.len(),
        user_ids.len(),
        new_time
    );
    Ok(htmx_refresh().into_response())
}

// ─── Route: GET /dev/users ──────────────────────────────────────────────────

pub async fn dev_list_users(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<Vec<UserSummary>>, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    let users_raw = state
        .repos
        .users
        .list_basic(user.league_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?;

    let users: Vec<UserSummary> = users_raw
        .into_iter()
        .map(|u| UserSummary {
            id: u.id,
            name: u.name,
            is_admin: false,
        })
        .collect();

    Ok(Json(users))
}

// ─── Route: POST /dev/switch-user/:id ───────────────────────────────────────

pub async fn dev_switch_user(
    State(state): State<AppState>,
    current: AuthenticatedUser,
    Path(target_id): Path<Uuid>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled".to_string()));
    }

    let user = state
        .repos
        .users
        .find_full_by_id(target_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    if user.league_id != current.league_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Cross-league user switch denied".to_string(),
        ));
    }

    let cookie = format!("pila_token={}; Path=/; HttpOnly; SameSite=Lax", user.token);

    Ok((
        [(axum::http::header::SET_COOKIE, cookie)],
        Redirect::to("/"),
    ))
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{find_next_unstarted_matchday, random_result};
    use crate::repo::fixture::IndexMatchRow;
    use crate::stage::Stage;
    use chrono::{DateTime, TimeZone, Utc};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn make_match(id: i32, kickoff: DateTime<Utc>) -> IndexMatchRow {
        IndexMatchRow {
            id,
            stage: Stage::Group,
            group_letter: Some("A".to_string()),
            kickoff_time: Some(kickoff),
            status: "scheduled".to_string(),
            score_home: None,
            score_away: None,
            team_home_id: Some(1),
            team_away_id: Some(2),
            home_name: "Home".to_string(),
            away_name: "Away".to_string(),
            home_flag: None,
            away_flag: None,
            predicted_home: None,
            predicted_away: None,
        }
    }

    #[test]
    fn find_next_matchday_groups_same_berlin_day_excludes_next_day() {
        // 12:00 UTC = 14:00 CEST and 18:00 UTC = 20:00 CEST — same Berlin day.
        // 13:00 UTC next day is the following Berlin day.
        let day1_early = Utc.with_ymd_and_hms(2026, 6, 12, 12, 0, 0).unwrap();
        let day1_late = Utc.with_ymd_and_hms(2026, 6, 12, 18, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 6, 13, 13, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 6, 12, 10, 0, 0).unwrap();

        let matches = vec![
            make_match(1, day1_early),
            make_match(2, day1_late),
            make_match(3, day2),
        ];
        let matchday = find_next_unstarted_matchday(&matches, now).unwrap();

        assert_eq!(matchday.len(), 2, "only same-day matches expected");
        assert!(
            matchday.iter().all(|m| m.id != 3),
            "next-day match must be excluded"
        );
    }

    #[test]
    fn random_result_values_in_range() {
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..200 {
            let (home, away) = random_result(&mut rng);
            assert!((0..=5).contains(&home), "home {home} out of range");
            assert!((0..=5).contains(&away), "away {away} out of range");
        }
    }
}
