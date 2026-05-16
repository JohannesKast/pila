//! Profile-jersey customisation. The `jersey_post` route is the one place
//! where we issue an HTMX out-of-band swap to refresh the user's row in
//! the leaderboard sidebar without re-rendering the page.

use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::services::fetch_leaderboard;
use crate::handlers::util::{html_escape, render_template, t_err, HandlerError};
use crate::translations::T;
use crate::views::{JerseyOption, LeaderboardEntry};
use crate::AppState;

#[derive(Template)]
#[template(path = "jersey_picker.html")]
struct JerseyPickerTemplate {
    grouped_options: Vec<(String, Vec<JerseyOption>)>,  // (variant_label, options)
    current: String,
    t: T,
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
    let t = crate::handlers::util::t_for(&state, &user.language);

    // Group jerseys by variant
    let mut home: Vec<JerseyOption> = Vec::new();
    let mut away: Vec<JerseyOption> = Vec::new();
    let mut fan: Vec<JerseyOption> = Vec::new();

    for (k, v) in state.jerseys.iter() {
        let display_name = t.get(&format!("jersey-name-{k}"));
        let option = JerseyOption {
            key: k.clone(),
            preset: v.clone(),
            display_name,
        };
        match v.variant {
            crate::jersey::JerseyVariant::Home => home.push(option),
            crate::jersey::JerseyVariant::Away => away.push(option),
            crate::jersey::JerseyVariant::Fan => fan.push(option),
        }
    }

    // Sort each group: Pila first, then by group, then by display name
    let sort_options = |opts: &mut Vec<JerseyOption>| {
        opts.sort_by(|a, b| {
            let pila_a = a.preset.group == "Pila";
            let pila_b = b.preset.group == "Pila";
            pila_b
                .cmp(&pila_a)
                .then_with(|| a.preset.group.cmp(&b.preset.group))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
    };
    sort_options(&mut home);
    sort_options(&mut away);
    sort_options(&mut fan);

    let grouped_options = vec![
        (t.get("jersey-variant-home"), home),
        (t.get("jersey-variant-away"), away),
        (t.get("jersey-variant-fan"), fan),
    ];

    let template = JerseyPickerTemplate {
        grouped_options,
        current: user.jersey_preset,
        t,
    };
    render_template(&template).unwrap_or_else(|_| Html("Internal error".to_string()))
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
) -> Result<Html<String>, HandlerError> {
    let lang = &user.language;
    if !state.jerseys.contains_key(&q.preset) {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-unknown-jersey",
        ));
    }
    state
        .repos
        .users
        .set_jersey(user.id, &q.preset)
        .await
        .map_err(|_| {
            t_err(
                &state,
                lang,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })?;

    let leaderboard = fetch_leaderboard(
        &state.repos,
        &state.jerseys,
        user.league_id,
        crate::time::now(&state.mock_now),
    )
    .await;
    let user_rank = leaderboard
        .iter()
        .position(|e| e.name == user.name)
        .map(|p| p + 1)
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "User not in leaderboard".to_string(),
        ))?;
    let user_entry = leaderboard[user_rank - 1].clone();

    let entry_template = LeaderboardEntryTemplate {
        entry: user_entry,
        rank: user_rank,
    };
    let entry_html = entry_template.render().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Template render error".to_string(),
        )
    })?;

    let oob_html = format!(
        r#"<div style="display:flex; align-items:center; gap:10px; padding:12px 14px; border-bottom:1px solid var(--pl-line); background:rgba(255,230,0,.04)" hx-swap-oob="innerHTML" id="leaderboard-entry-{}">{}</div>"#,
        html_escape(&user.name.to_lowercase()),
        entry_html
    );

    Ok(Html(oob_html))
}

const VALID_LOCALES: &[&str] = &["de", "en", "es", "fr"];

#[derive(Deserialize)]
pub struct SetLanguageForm {
    pub language: String,
}

/// `POST /profile/language` — persist user's language preference.
/// Responds with `HX-Location: /` so HTMX performs a client-side navigation
/// (full re-render in the chosen language, no browser reload flash).
pub async fn set_language_post(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Form(form): Form<SetLanguageForm>,
) -> impl IntoResponse {
    if !VALID_LOCALES.contains(&form.language.as_str()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    state
        .repos
        .users
        .set_language(user.id, &form.language)
        .await
        .ok();

    let mut headers = HeaderMap::new();
    headers.insert(
        "HX-Location",
        axum::http::HeaderValue::from_static("/"),
    );
    (StatusCode::OK, headers).into_response()
}
