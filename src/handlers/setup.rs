// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! First-run admin creation. Idempotent: if any user exists, the routes
//! redirect / refuse — preventing accidental admin overwrites.
//!
//! The form also creates the *first league* and its initial settings in
//! the same transaction as the first user — the app intentionally has no
//! seeded "Default" league, so a fresh deploy cannot end up with a user
//! belonging to nowhere.

use askama::Template;
use axum::{
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::handlers::util::{
    build_magic_link, make_login_cookie, render_template, t_err_from_headers, HandlerError,
};
use crate::notifier::{self, signal_configured};
use crate::repo::league::LeagueConfig;
use crate::repo::{FirstLeagueParams, Repos};
use crate::AppState;

#[derive(Template)]
#[template(path = "setup.html")]
struct SetupTemplate {
    lang_code: &'static str,
}

#[derive(Template)]
#[template(path = "setup_done.html")]
struct SetupDoneTemplate {
    lang_code: &'static str,
    name: String,
    magic_link: String,
}

async fn user_count(
    state: &AppState,
    headers: &HeaderMap,
    repos: &Repos,
) -> Result<i64, HandlerError> {
    repos.users.count().await.map_err(|_| {
        t_err_from_headers(
            state,
            headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    })
}

pub async fn setup_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, HandlerError> {
    if user_count(&state, &headers, &state.repos).await? > 0 {
        return Ok(Redirect::to("/").into_response());
    }
    let template = SetupTemplate { lang_code: "de" };
    Ok(render_template(&template)?.into_response())
}

const VALID_LOCALES: &[&str] = &["de", "en", "es", "fr"];

#[derive(Deserialize)]
pub struct SetupForm {
    pub name: String,
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub phone_number: String,
    #[serde(default)]
    pub email: String,
    pub league_name: String,
    #[serde(default)]
    pub default_language: String,
    #[serde(default)]
    pub signal_group_id: String,
    #[serde(default)]
    pub signal_from_number: String,
    #[serde(default)]
    pub rss_feed_url: String,
}

pub async fn setup_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Form(form): Form<SetupForm>,
) -> Result<(CookieJar, Response), HandlerError> {
    let err = |status: StatusCode, key: &str| t_err_from_headers(&state, &headers, status, key);
    let db_err = || err(StatusCode::INTERNAL_SERVER_ERROR, "error-database");

    if user_count(&state, &headers, &state.repos).await? > 0 {
        return Err(err(StatusCode::FORBIDDEN, "error-setup-already-done"));
    }
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "error-name-empty"));
    }
    // The real name is private and optional on the form: when left blank it
    // defaults to the tip name, matching the production backfill default.
    let real_name = form.real_name.trim();
    let real_name = if real_name.is_empty() {
        name.clone()
    } else {
        real_name.to_string()
    };
    let league_name = form.league_name.trim().to_string();
    if league_name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "error-league-name-empty"));
    }
    if league_name.len() > 255 {
        return Err(err(StatusCode::BAD_REQUEST, "error-league-name-too-long"));
    }
    let lang = form.default_language.trim();
    let lang = if lang.is_empty() { "de" } else { lang };
    if !VALID_LOCALES.contains(&lang) {
        return Err(err(StatusCode::BAD_REQUEST, "error-unknown-language"));
    }

    let phone = form.phone_number.trim().to_string();
    let phone_opt: Option<&str> = if phone.is_empty() { None } else { Some(&phone) };
    let email = form.email.trim().to_string();
    let email_opt: Option<&str> = if email.is_empty() { None } else { Some(&email) };

    let id = Uuid::new_v4();
    let token = Uuid::new_v4().to_string();
    let magic_link = build_magic_link(&token, &state.base_url);

    let settings = [
        (LeagueConfig::KEY_DEFAULT_LANGUAGE, lang),
        (
            LeagueConfig::KEY_SIGNAL_GROUP_ID,
            form.signal_group_id.trim(),
        ),
        (
            LeagueConfig::KEY_SIGNAL_FROM_NUMBER,
            form.signal_from_number.trim(),
        ),
        (LeagueConfig::KEY_RSS_FEED_URL, form.rss_feed_url.trim()),
    ];
    state
        .repos
        .bootstrap
        .create_first_league_and_admin(FirstLeagueParams {
            user_id: id,
            user_name: &name,
            user_real_name: &real_name,
            token: &token,
            phone_number: phone_opt,
            email: email_opt,
            language: lang,
            league_name: &league_name,
            settings: &settings,
        })
        .await
        .map_err(|e| {
            tracing::error!(%e, "setup: bootstrap transaction failed");
            db_err()
        })?;

    // Signal invite is outside the transaction — best-effort side effect.
    let invite_t = crate::handlers::util::t_for(&state, lang);
    if let Some(p) = phone_opt {
        if signal_configured(&state.signal_api_url, &state.signal_from_number) {
            if let Err(e) = notifier::send_invite_via_signal(
                p,
                &name,
                &magic_link,
                &state.signal_api_url,
                &state.signal_from_number,
                &invite_t,
            )
            .await
            {
                tracing::warn!("setup: Signal invite to {p} failed: {e}");
            }
        }
    }
    // Email invite — also best-effort.
    if let Some(e) = email_opt {
        if let Some(ref smtp) = state.smtp_config {
            if let Err(err) =
                crate::mail::send_invite_email(smtp, &name, e, &magic_link, &invite_t).await
            {
                tracing::warn!("setup: email invite to {e} failed: {err}");
            }
        }
    }

    let updated_jar = jar.add(make_login_cookie(token));
    let page = SetupDoneTemplate {
        lang_code: lang_static(lang),
        magic_link,
        name,
    };
    Ok((updated_jar, render_template(&page)?.into_response()))
}

fn lang_static(lang: &str) -> &'static str {
    match lang {
        "en" => "en",
        "es" => "es",
        "fr" => "fr",
        _ => "de",
    }
}
