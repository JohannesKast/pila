// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Standalone leaderboard page (`GET /leaderboard`).

use askama::Template;
use axum::{extract::State, response::Html};

use crate::auth::AuthenticatedUser;
use crate::handlers::services::fetch_leaderboard;
use crate::handlers::util::{league_scope_path, render_template};
use crate::translations::T;
use crate::views::LeaderboardEntry;
use crate::AppState;

#[derive(Template)]
#[template(path = "leaderboard.html")]
struct LeaderboardTemplate {
    entries: Vec<LeaderboardEntry>,
    scope_path: String,
    t: T,
    lang_code: String,
    dev_mode: bool,
}

pub async fn leaderboard(State(state): State<AppState>, user: AuthenticatedUser) -> Html<String> {
    let now = crate::time::now(&state.mock_now);
    let entries = fetch_leaderboard(&state.repos, &state.jerseys, user.league_id, now).await;
    let lang_code = user.language.clone();
    let t = crate::handlers::util::t_for(&state, &user.language);
    let template = LeaderboardTemplate {
        entries,
        scope_path: league_scope_path(user.league_id),
        t,
        lang_code,
        dev_mode: state.dev_mode,
    };
    render_template(&template).unwrap_or_else(|_| Html("Internal error".to_string()))
}
