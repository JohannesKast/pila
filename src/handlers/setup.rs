//! First-run admin creation. Idempotent: if any user exists, the routes
//! redirect / refuse — preventing accidental admin overwrites.

use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::handlers::util::{build_magic_link, make_login_cookie};
use crate::notifier;
use crate::repo::{self, Repos, DEFAULT_LEAGUE_ID};
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

async fn user_count(repos: &Repos) -> Result<i64, (StatusCode, &'static str)> {
    repos
        .users
        .count()
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))
}

pub async fn setup_get(
    State(state): State<AppState>,
) -> Result<Response, (StatusCode, &'static str)> {
    if user_count(&state.repos).await? > 0 {
        return Ok(Redirect::to("/").into_response());
    }
    let template = SetupTemplate { lang_code: "de" };
    Ok(Html(template.render().unwrap()).into_response())
}

#[derive(Deserialize)]
pub struct SetupForm {
    name: String,
    #[serde(default)]
    phone_number: String,
}

pub async fn setup_post(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<SetupForm>,
) -> Result<(CookieJar, Response), (StatusCode, &'static str)> {
    if user_count(&state.repos).await? > 0 {
        return Err((StatusCode::FORBIDDEN, "Setup bereits abgeschlossen."));
    }
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name darf nicht leer sein."));
    }
    let phone = form.phone_number.trim().to_string();
    let phone_opt: Option<&str> = if phone.is_empty() { None } else { Some(&phone) };

    let id = Uuid::new_v4();
    let token = Uuid::new_v4().to_string();
    let magic_link = build_magic_link(&token);

    state
        .repos
        .users
        .create(repo::user::NewUser {
            id,
            name: &name,
            token: &token,
            is_admin: true,
            phone_number: phone_opt,
            league_id: DEFAULT_LEAGUE_ID,
            language: "de",
        })
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    // First admin during setup gets full multi-league powers — without this,
    // a fresh deploy would have no path to ever create a second league.
    state
        .repos
        .users
        .set_can_create_league(id, true)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    if let Some(p) = phone_opt {
        if notifier::signal_configured() {
            if let Err(e) = notifier::send_invite_via_signal(p, &name, &magic_link).await {
                tracing::warn!("Setup: Signal-Einladung an {p} fehlgeschlagen: {e}");
            }
        }
    }

    let updated_jar = jar.add(make_login_cookie(token));
    let page = SetupDoneTemplate {
        lang_code: "de",
        magic_link,
        name,
    };
    Ok((updated_jar, Html(page.render().unwrap()).into_response()))
}
