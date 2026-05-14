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

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect},
};
use chrono::{NaiveDateTime, TimeZone, Utc};
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
fn random_goals(rng: &mut StdRng) -> i32 {
    let r: f64 = rng.gen();
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
) -> Result<Html<String>, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }
    render_panel(&state, &user).await
}

/// Renders the dev panel fragment. Used by GET /dev and as the HTMX response
/// of every mutation handler so the panel stays open after a form submit.
async fn render_panel(
    state: &AppState,
    user: &AuthenticatedUser,
) -> Result<Html<String>, (StatusCode, &'static str)> {
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
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

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
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

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

    render_template(&template).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Template error"))
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
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }

    // The HTML <input type="datetime-local"> field emits values like
    // `2026-06-12T18:00` (no seconds, no timezone) — not RFC3339. Parse as
    // a naive local-wallclock value, then treat it as UTC.
    let naive = NaiveDateTime::parse_from_str(&form.time, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(&form.time, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid datetime format (use YYYY-MM-DDTHH:MM)",
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
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
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
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }

    let matches = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let now = time::now(&state.mock_now);
    let mut rng = StdRng::from_entropy();

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
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
    }

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/tips/all-users ────────────────────────────────────────

pub async fn dev_random_tips_all_users(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    _form: Form<EmptyForm>,
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }

    let user_ids = state
        .repos
        .users
        .list_ids(user.league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let now = time::now(&state.mock_now);
    let mut rng = StdRng::from_entropy();

    for uid in user_ids {
        let matches = state
            .repos
            .matches
            .list_for_index(uid)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

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
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
        }
    }

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/results/random ────────────────────────────────────────

pub async fn dev_random_results(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    _form: Form<EmptyForm>,
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }

    let matches = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let now = time::now(&state.mock_now);
    let mut rng = StdRng::from_entropy();

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
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
    }

    Ok(htmx_refresh().into_response())
}

// ─── Route: POST /dev/simulate/next-matchday ─────────────────────────────────

pub async fn dev_simulate_next_matchday(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    _form: Form<EmptyForm>,
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }

    let matches = state
        .repos
        .matches
        .list_for_index(user.id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let now = time::now(&state.mock_now);

    // Earliest unstarted match with both teams known defines the matchday.
    let next_match = matches
        .iter()
        .filter(|m| m.team_home_id.is_some() && m.team_away_id.is_some())
        .filter(|m| m.kickoff_time.is_some_and(|k| k > now))
        .min_by_key(|m| m.kickoff_time);

    let Some(next_match) = next_match else {
        tracing::warn!(
            "dev_simulate_next_matchday: no future tippable match found (mock_now={}, matches loaded={})",
            now,
            matches.len()
        );
        return Err((
            StatusCode::BAD_REQUEST,
            "Kein zukünftiges Spiel gefunden. Matches-Tabelle leer oder Mock-Zeit nach Finale?",
        ));
    };

    // Group matches by calendar day in Europe/Berlin — that is what users
    // perceive as a "Spieltag".
    use chrono_tz::Europe::Berlin;
    let matchday = next_match
        .kickoff_time
        .unwrap()
        .with_timezone(&Berlin)
        .date_naive();

    let matchday_matches: Vec<_> = matches
        .iter()
        .filter(|m| m.team_home_id.is_some() && m.team_away_id.is_some())
        .filter(|m| {
            m.kickoff_time
                .is_some_and(|k| k > now && k.with_timezone(&Berlin).date_naive() == matchday)
        })
        .collect();

    let mut rng = StdRng::from_entropy();

    // 1. BEFORE jumping time: generate random tips for every user on every
    //    matchday match. Existing tips of the current user are preserved so
    //    the tester can compare their own picks against the random crowd.
    let user_ids = state
        .repos
        .users
        .list_ids(user.league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    for m in &matchday_matches {
        for uid in &user_ids {
            // Preserve manual tip of the current user; randomize everyone else.
            if *uid == user.id && m.predicted_home.is_some() {
                continue;
            }
            let home = random_goals(&mut rng);
            let away = random_goals(&mut rng);
            state
                .repos
                .predictions
                .upsert(*uid, m.id, home, away)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
        }
    }

    // 2. Jump mock time to the end of the matchday (latest kickoff + 2h)
    //    so every match of that day is now "in the past" from the app's view.
    let latest_kickoff = matchday_matches
        .iter()
        .filter_map(|m| m.kickoff_time)
        .max()
        .unwrap_or_else(|| next_match.kickoff_time.unwrap());
    let new_time = latest_kickoff + chrono::Duration::hours(2);
    time::set_mock_time(&state.mock_now, new_time);

    // 3. Force every matchday match to a finished state. Preserves existing
    //    scores (so repeated clicks don't keep re-rolling) but always sets
    //    `status = "finished"` — necessary because the ESPN one-shot sync at
    //    startup reverts the status of any previously-finished dev match
    //    back to "scheduled" (its upsert uses `status = EXCLUDED.status`,
    //    not COALESCE). Without forcing it here, those matches would render
    //    as locked-but-unfinished ("– : –") on the index.
    tracing::info!("🎮 Updating {} matchday matches with results", matchday_matches.len());
    for m in &matchday_matches {
        let home = random_goals(&mut rng);
        let away = random_goals(&mut rng);
        tracing::info!("🎮 Setting result: match {} ({} vs {}) → {}:{}", m.id, m.home_name, m.away_name, home, away);
        state
            .repos
            .matches
            .update_result(m.id, Some(home), Some(away), "finished")
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
    }

    tracing::info!(
        "🎮 Simulated matchday {} ({} matches, {} users), mock_now is now: {}",
        matchday,
        matchday_matches.len(),
        user_ids.len(),
        new_time
    );

    Ok(htmx_refresh().into_response())
}

// ─── Route: GET /dev/users ──────────────────────────────────────────────────

pub async fn dev_list_users(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<Vec<UserSummary>>, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }

    let users_raw = state
        .repos
        .users
        .list_basic(user.league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

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
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    if !state.dev_mode {
        return Err((StatusCode::NOT_FOUND, "Dev mode not enabled"));
    }

    let user = state
        .repos
        .users
        .find_full_by_id(target_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::NOT_FOUND, "User not found"))?;

    if user.league_id != current.league_id {
        return Err((StatusCode::FORBIDDEN, "Cross-league user switch denied"));
    }

    let cookie = format!(
        "pila_token={}; Path=/; HttpOnly; SameSite=Lax",
        user.token
    );

    Ok((
        [(axum::http::header::SET_COOKIE, cookie)],
        Redirect::to("/"),
    ))
}
