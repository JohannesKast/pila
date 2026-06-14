// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Self-service profile editing — lets a player change their own names at any
//! time without admin help.
//!
//! Two names are stored per user (see the `users` table / `20260614…`
//! migration):
//!
//! - the public **tip name** (`name`) shown on the leaderboard and in other
//!   players' comments. Unique per league, so a change can collide.
//! - the private **real name** (`real_name`) which only league admins ever
//!   see. No uniqueness constraint.
//!
//! The editor renders into the shared bottom `#sheet`, mirroring the jersey
//! picker. On a successful save we answer with `HX-Location` pointing at the
//! league-scoped dashboard so HTMX does a flash-free client-side navigation
//! and the new tip name shows up in the topbar and leaderboard immediately.

use askama::Template;
use axum::{
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::util::{league_scope_path, render_template};
use crate::repo;
use crate::translations::T;
use crate::AppState;

#[derive(Template)]
#[template(path = "profile_sheet.html")]
struct ProfileSheetTemplate {
    scope_path: String,
    name: String,
    real_name: String,
    /// Pre-translated inline error, empty when the sheet opens cleanly.
    error: String,
    t: T,
}

fn render_sheet(
    state: &AppState,
    lang: &str,
    league_id: uuid::Uuid,
    name: String,
    real_name: String,
    error: String,
) -> Html<String> {
    let tpl = ProfileSheetTemplate {
        scope_path: league_scope_path(league_id),
        name,
        real_name,
        error,
        t: crate::handlers::util::t_for(state, lang),
    };
    render_template(&tpl).unwrap_or_else(|_| Html("Internal error".to_string()))
}

/// `GET /profile/name-editor` — open the profile sheet pre-filled with the
/// user's current names.
pub async fn profile_editor_get(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Html<String> {
    render_sheet(
        &state,
        &user.language,
        user.league_id,
        user.name,
        user.real_name,
        String::new(),
    )
}

#[derive(Deserialize)]
pub struct ProfileNameForm {
    pub name: String,
    #[serde(default)]
    pub real_name: String,
}

/// `POST /profile/name` — persist the user's tip name and real name.
///
/// On validation failure (empty / taken tip name) the sheet is re-rendered
/// with an inline error and the user's entered values, so nothing is lost. On
/// success we send a scoped `HX-Location` to re-render the page in place.
pub async fn profile_name_post(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Form(form): Form<ProfileNameForm>,
) -> impl IntoResponse {
    let lang = user.language.clone();
    let t = crate::handlers::util::t_for(&state, &lang);

    let name = form.name.trim().to_string();
    // The real name is private; blank keeps it equal to the tip name.
    let real_name_in = form.real_name.trim();
    let real_name = if real_name_in.is_empty() {
        name.clone()
    } else {
        real_name_in.to_string()
    };

    let sheet_err = |key: &str, name: String, real_name: String| {
        render_sheet(&state, &lang, user.league_id, name, real_name, t.get(key)).into_response()
    };

    if name.is_empty() {
        return sheet_err("error-name-empty", name, real_name);
    }
    if name.chars().count() > 255 || real_name.chars().count() > 255 {
        return sheet_err("error-name-empty", name, real_name);
    }

    // Renaming to the user's own current name is a no-op update that the unique
    // index accepts; only a genuine collision with another player surfaces as
    // `RepoError::Conflict`.
    match state.repos.users.rename(user.id, &name).await {
        Ok(()) => {}
        Err(repo::RepoError::Conflict) => {
            return sheet_err("error-name-taken", name, real_name);
        }
        Err(_) => return sheet_err("error-database", name, real_name),
    }

    if state
        .repos
        .users
        .set_real_name(user.id, &real_name)
        .await
        .is_err()
    {
        return sheet_err("error-database", name, real_name);
    }

    let mut headers = HeaderMap::new();
    if let Ok(location) = axum::http::HeaderValue::from_str(&league_scope_path(user.league_id)) {
        headers.insert("HX-Location", location);
    }
    (StatusCode::OK, headers).into_response()
}
