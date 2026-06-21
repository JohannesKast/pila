// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Profile-jersey customisation. The `jersey_post` route is the one place
//! where we issue an HTMX out-of-band swap to refresh the user's row in
//! the leaderboard sidebar without re-rendering the page.

use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::services::{build_badge_context, fetch_leaderboard};
use crate::handlers::util::{
    html_escape, league_scope_path, make_theme_cookie, render_template, t_err, HandlerError,
};
use crate::translations::T;
use crate::views::{JerseyOption, LeaderboardEntry};
use crate::AppState;

#[derive(Template)]
#[template(path = "jersey_picker.html")]
struct JerseyPickerTemplate {
    grouped_options: Vec<(String, Vec<JerseyOption>)>, // (variant_label, options)
    current: String,
    scope_path: String,
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
        scope_path: league_scope_path(user.league_id),
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

    let now = crate::time::now(&state.mock_now);
    let leaderboard = fetch_leaderboard(&state.repos, &state.jerseys, user.league_id, now).await;
    let user_rank = leaderboard
        .iter()
        .position(|e| e.name == user.name)
        .map(|p| p + 1)
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "User not in leaderboard".to_string(),
        ))?;
    let mut user_entry = leaderboard[user_rank - 1].clone();

    // The leaderboard row keeps the user's badge chips inline, so the OOB
    // refresh must recompute them — fetch_leaderboard leaves them empty.
    let t = crate::handlers::util::t_for(&state, &user.language);
    let badge_ctx = build_badge_context(&state.repos, user.id, user.league_id, now).await;
    user_entry.achievements = crate::badges::achievement_badges_for(&badge_ctx.as_ctx(), &t);

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
/// Responds with `HX-Location` so HTMX performs a client-side navigation
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
    if let Ok(value) = axum::http::HeaderValue::from_str(&league_scope_path(user.league_id)) {
        headers.insert("HX-Location", value);
    }
    (StatusCode::OK, headers).into_response()
}

const VALID_THEMES: &[&str] = &["dark", "light"];

#[derive(Deserialize)]
pub struct SetThemeForm {
    pub theme: String,
}

/// `POST /profile/theme` — persist the user's colour theme.
///
/// The topbar toggle already flips the theme and the `pila_theme` cookie
/// client-side for instant, flash-free feedback; this request mirrors the
/// choice into the database so it follows the user across devices. We also
/// (re)set the cookie server-side so it stays authoritative. No body is
/// returned — the page does not need to re-render.
pub async fn set_theme_post(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    jar: CookieJar,
    Form(form): Form<SetThemeForm>,
) -> impl IntoResponse {
    if !VALID_THEMES.contains(&form.theme.as_str()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    state.repos.users.set_theme(user.id, &form.theme).await.ok();
    let jar = jar.add(make_theme_cookie(form.theme));
    (jar, StatusCode::OK).into_response()
}
