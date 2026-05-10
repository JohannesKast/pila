//! Magic-link login. The token cookie is the only authentication
//! mechanism in the system — `create_invite.sh` is the issuer.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Redirect,
};
use axum_extra::extract::CookieJar;

use crate::handlers::util::make_login_cookie;
use crate::AppState;

pub async fn login_magic_link(
    State(state): State<AppState>,
    Path(token): Path<String>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), (StatusCode, &'static str)> {
    let exists = state
        .repos
        .users
        .find_by_token(&token)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .is_some();

    if exists {
        let updated_jar = jar.add(make_login_cookie(token));
        Ok((updated_jar, Redirect::to("/")))
    } else {
        Err((StatusCode::UNAUTHORIZED, "Ungültiger oder abgelaufener Link."))
    }
}
