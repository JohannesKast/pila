//! Standalone leaderboard page (`GET /leaderboard`).

use askama::Template;
use axum::{extract::State, response::Html};

use crate::auth::AuthenticatedUser;
use crate::handlers::services::fetch_leaderboard;
use crate::views::LeaderboardEntry;
use crate::AppState;

#[derive(Template)]
#[template(path = "leaderboard.html")]
struct LeaderboardTemplate {
    entries: Vec<LeaderboardEntry>,
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
