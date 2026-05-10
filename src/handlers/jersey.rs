//! Profile-jersey customisation. The `jersey_post` route is the one place
//! where we issue an HTMX out-of-band swap to refresh the user's row in
//! the leaderboard sidebar without re-rendering the page.

use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::services::fetch_leaderboard;
use crate::handlers::util::html_escape;
use crate::views::{JerseyOption, LeaderboardEntry};
use crate::AppState;

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
    Query(q): Query<JerseyPostQuery>,
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
    let entry_html = entry_template
        .render()
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Template render error"))?;

    let oob_html = format!(
        r#"<div style="display:flex; align-items:center; gap:10px; padding:12px 14px; border-bottom:1px solid var(--pl-line); background:rgba(255,230,0,.04)" hx-swap-oob="innerHTML" id="leaderboard-entry-{}">{}</div>"#,
        html_escape(&user.name.to_lowercase()),
        entry_html
    );

    Ok(Html(oob_html))
}
