// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use crate::handlers::util::{
    league_id_from_path, scoped_login_cookie_name, t_err_from_headers, HandlerError,
    LEGACY_LOGIN_COOKIE,
};
use crate::AppState;

pub struct AuthenticatedUser {
    pub id: Uuid,
    pub name: String,
    /// Private real first name — only ever rendered back to the user in their
    /// own profile editor, never exposed to other players.
    pub real_name: String,
    pub is_admin: bool,
    /// Permission to create new leagues. A regular admin manages their own
    /// league; a super-admin (`can_create_league = true`) can spawn fresh
    /// leagues and manage settings on any league.
    pub can_create_league: bool,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub jersey_preset: String,
    pub language: String,
    /// Tenancy boundary — every aggregate query in handlers must filter by
    /// this id. Set when the user is created (via `/setup` or the per-league
    /// admin form); there is no seeded "Default" league.
    pub league_id: Uuid,
}

async fn lookup_user(
    state: &AppState,
    token: &str,
) -> Result<Option<AuthenticatedUser>, crate::repo::RepoError> {
    let row = state.repos.users.find_by_token(token).await?;
    Ok(row.map(|u| AuthenticatedUser {
        id: u.id,
        name: u.name,
        real_name: u.real_name,
        is_admin: u.is_admin,
        can_create_league: u.can_create_league,
        phone_number: u.phone_number,
        email: u.email,
        jersey_preset: u.jersey_preset,
        language: u.language,
        league_id: u.league_id,
    }))
}

async fn lookup_scoped_user(
    state: &AppState,
    parts: &Parts,
) -> Result<Option<AuthenticatedUser>, crate::repo::RepoError> {
    let cookie_jar = CookieJar::from_headers(&parts.headers);
    let path = parts.uri.path();
    let scoped_league_id = league_id_from_path(path);

    if let Some(league_id) = scoped_league_id {
        let scoped_name = scoped_login_cookie_name(league_id);
        for token in [
            cookie_jar.get(&scoped_name).map(|c| c.value().to_string()),
            cookie_jar
                .get(LEGACY_LOGIN_COOKIE)
                .map(|c| c.value().to_string()),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(user) = lookup_user(state, &token).await? {
                if user.league_id == league_id {
                    return Ok(Some(user));
                }
            }
        }
        return Ok(None);
    }

    if path.starts_with("/l/") {
        return Ok(None);
    }

    let token = cookie_jar
        .get(LEGACY_LOGIN_COOKIE)
        .map(|c| c.value().to_string());
    match token {
        Some(token) => lookup_user(state, &token).await,
        None => Ok(None),
    }
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = HandlerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = lookup_scoped_user(state, parts).await.map_err(|_| {
            t_err_from_headers(
                state,
                &parts.headers,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })?;
        if let Some(u) = user {
            return Ok(u);
        }

        Err(t_err_from_headers(
            state,
            &parts.headers,
            StatusCode::UNAUTHORIZED,
            "error-not-authenticated",
        ))
    }
}

pub struct MaybeAuthenticatedUser(pub Option<AuthenticatedUser>);

impl FromRequestParts<AppState> for MaybeAuthenticatedUser {
    type Rejection = HandlerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = lookup_scoped_user(state, parts).await.map_err(|_| {
            t_err_from_headers(
                state,
                &parts.headers,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })?;
        Ok(MaybeAuthenticatedUser(user))
    }
}

pub struct AdminUser(pub AuthenticatedUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = HandlerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err(crate::handlers::util::t_err(
                state,
                &user.language,
                StatusCode::FORBIDDEN,
                "error-admin-required",
            ));
        }
        Ok(AdminUser(user))
    }
}

/// Admin who additionally has the right to create new leagues. Distinct from
/// `AdminUser` so league CRUD routes can refuse a regular league admin.
pub struct SuperAdminUser(pub AuthenticatedUser);

impl FromRequestParts<AppState> for SuperAdminUser {
    type Rejection = HandlerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;
        if !user.is_admin || !user.can_create_league {
            return Err(crate::handlers::util::t_err(
                state,
                &user.language,
                StatusCode::FORBIDDEN,
                "error-super-admin-required",
            ));
        }
        Ok(SuperAdminUser(user))
    }
}
