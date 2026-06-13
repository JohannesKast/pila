// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Magic-link login. The token cookie is the only authentication
//! mechanism in the system — `create_invite.sh` is the issuer.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
};
use axum_extra::extract::CookieJar;

use crate::handlers::util::{
    make_csrf_cookie, make_login_cookie, make_theme_cookie, t_err_from_headers, HandlerError,
};
use crate::AppState;

pub async fn login_magic_link(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), HandlerError> {
    let user = state.repos.users.find_by_token(&token).await.map_err(|_| {
        t_err_from_headers(
            &state,
            &headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    })?;

    if let Some(user) = user {
        // Seed the theme cookie from the stored preference so the chosen
        // theme follows the user onto a freshly logged-in device.
        let updated_jar = jar
            .add(make_login_cookie(token.clone()))
            .add(make_csrf_cookie(token))
            .add(make_theme_cookie(user.theme));
        Ok((updated_jar, Redirect::to("/")))
    } else {
        Err(t_err_from_headers(
            &state,
            &headers,
            StatusCode::UNAUTHORIZED,
            "error-invalid-or-expired-link",
        ))
    }
}
