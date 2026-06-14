// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Public self-registration via a shareable invite link.
//!
//! Anyone holding `/join/{token}` can create their own user in the link's
//! league — no admin action per player required. The flow is intentionally
//! passwordless and mirrors `/setup`: on success the browser receives the
//! login + CSRF cookies and the new personal magic link is shown so the
//! player can bookmark it.
//!
//! Because the visitor has no session yet, the POST is exempt from the CSRF
//! middleware (see `csrf_middleware` in `main.rs`), just like `/setup` and the
//! magic-link login.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::handlers::util::{
    build_magic_link, league_scope_path, make_csrf_cookie, make_login_cookie,
    make_scoped_csrf_cookie, make_scoped_login_cookie, preferred_lang, render_template, t_err,
    t_err_from_headers, t_for, HandlerError,
};
use crate::repo;
use crate::translations::T;
use crate::AppState;

#[derive(Template)]
#[template(path = "join.html")]
struct JoinTemplate {
    lang_code: String,
    league_name: String,
    token: String,
    t: T,
}

#[derive(Template)]
#[template(path = "join_done.html")]
struct JoinDoneTemplate {
    lang_code: String,
    name: String,
    magic_link: String,
    continue_path: String,
    t: T,
}

/// Renders the registration form for a valid invite token. Unknown or revoked
/// tokens get a generic "invalid invite" error rather than leaking whether a
/// token ever existed.
pub async fn join_get(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<Response, HandlerError> {
    let lang = preferred_lang(&headers);
    let db_err = || {
        t_err_from_headers(
            &state,
            &headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let invite = state
        .repos
        .invites
        .find_by_token(&token)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(|| {
            t_err_from_headers(
                &state,
                &headers,
                StatusCode::NOT_FOUND,
                "error-invalid-invite",
            )
        })?;

    let league = state
        .repos
        .leagues
        .find_by_id(invite.league_id)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(|| {
            t_err_from_headers(
                &state,
                &headers,
                StatusCode::NOT_FOUND,
                "error-invalid-invite",
            )
        })?;

    let template = JoinTemplate {
        league_name: league.name,
        token,
        t: t_for(&state, &lang),
        lang_code: lang,
    };
    Ok(render_template(&template)?.into_response())
}

#[derive(Deserialize)]
pub struct JoinForm {
    pub name: String,
    #[serde(default)]
    pub real_name: String,
}

/// Creates the self-registered user in the invite's league, logs them in via
/// the cookie pair, and shows their personal magic link.
pub async fn join_post(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    Form(form): Form<JoinForm>,
) -> Result<(CookieJar, Response), HandlerError> {
    let req_lang = preferred_lang(&headers);
    let err = |status: StatusCode, key: &str| t_err(&state, &req_lang, status, key);
    let db_err = || err(StatusCode::INTERNAL_SERVER_ERROR, "error-database");

    let invite = state
        .repos
        .invites
        .find_by_token(&token)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "error-invalid-invite"))?;
    let league_id = invite.league_id;

    let name = form.name.trim();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "error-name-empty"));
    }
    if name.chars().count() > 255 {
        return Err(err(StatusCode::BAD_REQUEST, "error-name-empty"));
    }
    // Private real name; blank falls back to the tip name (production default).
    let real_name = form.real_name.trim();
    let real_name = if real_name.is_empty() {
        name
    } else {
        real_name
    };
    if real_name.chars().count() > 255 {
        return Err(err(StatusCode::BAD_REQUEST, "error-name-empty"));
    }

    // Reject duplicate display names up front so re-opening the invite link,
    // a lost magic link, or a double submit no longer spawns a second account
    // with the same name. The unique index on (league_id, lower(name)) is the
    // race-safe backstop and surfaces below as RepoError::Conflict.
    if state
        .repos
        .users
        .name_exists(league_id, name)
        .await
        .map_err(|_| db_err())?
    {
        return Err(err(StatusCode::CONFLICT, "error-name-taken"));
    }

    let cfg = state
        .repos
        .leagues
        .get_config(league_id)
        .await
        .map_err(|_| db_err())?;

    let id = Uuid::new_v4();
    let user_token = Uuid::new_v4().to_string();
    state
        .repos
        .users
        .create(repo::user::NewUser {
            id,
            name,
            real_name,
            token: &user_token,
            is_admin: false,
            phone_number: None,
            email: None,
            league_id,
            language: &cfg.default_language,
        })
        .await
        .map_err(|e| match e {
            repo::RepoError::Conflict => err(StatusCode::CONFLICT, "error-name-taken"),
            _ => db_err(),
        })?;

    let magic_link = build_magic_link(&user_token, &state.base_url);
    let updated_jar = jar
        .add(make_login_cookie(user_token.clone()))
        .add(make_csrf_cookie(user_token.clone()))
        .add(make_scoped_login_cookie(user_token.clone(), league_id))
        .add(make_scoped_csrf_cookie(user_token, league_id));

    let page = JoinDoneTemplate {
        lang_code: cfg.default_language.clone(),
        name: name.to_string(),
        magic_link,
        continue_path: league_scope_path(league_id),
        t: t_for(&state, &cfg.default_language),
    };
    Ok((updated_jar, render_template(&page)?.into_response()))
}
