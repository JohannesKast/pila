//! Standalone leaderboard page (`GET /leaderboard`).

use askama::Template;
use axum::{extract::State, response::Html};

use crate::auth::AuthenticatedUser;
use crate::handlers::services::fetch_leaderboard;
use crate::translations::T;
use crate::views::LeaderboardEntry;
use crate::AppState;

#[derive(Template)]
#[template(path = "leaderboard.html")]
struct LeaderboardTemplate {
    entries: Vec<LeaderboardEntry>,
    t: T,
    lang_code: String,
}

pub async fn leaderboard(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Html<String> {
    let now = chrono::Utc::now();
    let entries = fetch_leaderboard(&state.repos, &state.jerseys, now).await;
    let lang_code = user.language.clone();
    let t = state
        .translations
        .get(&user.language)
        .or_else(|| state.translations.get("de"))
        .expect("de locale always present")
        .clone();
    let template = LeaderboardTemplate { entries, t, lang_code };
    Html(template.render().unwrap())
}
