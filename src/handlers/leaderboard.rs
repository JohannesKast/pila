//! Standalone leaderboard page (`GET /leaderboard`).

use askama::Template;
use axum::{extract::State, response::Html};

use crate::auth::AuthenticatedUser;
use crate::handlers::services::fetch_leaderboard;
use crate::handlers::util::render_template;
use crate::translations::T;
use crate::views::LeaderboardEntry;
use crate::AppState;

#[derive(Template)]
#[template(path = "leaderboard.html")]
struct LeaderboardTemplate {
    entries: Vec<LeaderboardEntry>,
    t: T,
    lang_code: String,
    dev_mode: bool,
}

pub async fn leaderboard(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Html<String> {
    let now = crate::time::now(&state.mock_now);
    let entries = fetch_leaderboard(&state.repos, &state.jerseys, user.league_id, now).await;
    let lang_code = user.language.clone();
    let t = crate::handlers::util::t_for(&state, &user.language);
    let template = LeaderboardTemplate {
        entries,
        t,
        lang_code,
        dev_mode: state.dev_mode,
    };
    render_template(&template).unwrap_or_else(|_| Html("Internal error".to_string()))
}
