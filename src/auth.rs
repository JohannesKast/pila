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
}

#[async_trait]
impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let cookie_jar = CookieJar::from_headers(&parts.headers);
        let token = cookie_jar.get("pila_token").map(|c| c.value());

        if let Some(token) = token {
            let user_result = sqlx::query!(
                "SELECT id, name, is_admin FROM users WHERE token = $1",
                token
            )
            .fetch_optional(&state.db)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

            if let Some(user) = user_result {
                return Ok(AuthenticatedUser {
                    id: user.id,
                    name: user.name,
                    is_admin: user.is_admin,
                });
            }
        }

        Err((
            StatusCode::UNAUTHORIZED,
            "Nicht authentifiziert. Bitte nutze deinen Magic Link (z.B. /play/me/mein-token).",
        ))
    }
}
