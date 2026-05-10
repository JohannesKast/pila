use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use crate::AppState;

pub struct AuthenticatedUser {
    pub id: Uuid,
    pub name: String,
    pub is_admin: bool,
    pub phone_number: Option<String>,
    pub jersey_preset: String,
    pub language: String,
}

async fn lookup_user(
    state: &AppState,
    token: &str,
) -> Result<Option<AuthenticatedUser>, crate::repo::RepoError> {
    let row = state.repos.users.find_by_token(token).await?;
    Ok(row.map(|u| AuthenticatedUser {
        id: u.id,
        name: u.name,
        is_admin: u.is_admin,
        phone_number: u.phone_number,
        jersey_preset: u.jersey_preset,
        language: u.language,
    }))
}

#[async_trait]
impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let cookie_jar = CookieJar::from_headers(&parts.headers);
        let token = cookie_jar.get("pila_token").map(|c| c.value().to_string());

        if let Some(token) = token {
            let user = lookup_user(state, &token)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
            if let Some(u) = user {
                return Ok(u);
            }
        }

        Err((
            StatusCode::UNAUTHORIZED,
            "Nicht authentifiziert. Bitte nutze deinen Magic Link (z.B. /play/me/mein-token).",
        ))
    }
}

pub struct MaybeAuthenticatedUser(pub Option<AuthenticatedUser>);

#[async_trait]
impl FromRequestParts<AppState> for MaybeAuthenticatedUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let cookie_jar = CookieJar::from_headers(&parts.headers);
        let token = cookie_jar.get("pila_token").map(|c| c.value().to_string());

        if let Some(token) = token {
            let user = lookup_user(state, &token)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;
            return Ok(MaybeAuthenticatedUser(user));
        }
        Ok(MaybeAuthenticatedUser(None))
    }
}

pub struct AdminUser(pub AuthenticatedUser);

#[async_trait]
impl FromRequestParts<AppState> for AdminUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err((StatusCode::FORBIDDEN, "Admin-Rechte erforderlich."));
        }
        Ok(AdminUser(user))
    }
}
