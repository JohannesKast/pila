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
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::handlers::util::{build_magic_link, make_login_cookie};
use crate::notifier;
use crate::repo::league::LeagueConfig;
use crate::repo::{self, Repos};
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

const VALID_LOCALES: &[&str] = &["de", "en", "es", "fr"];

#[derive(Deserialize)]
pub struct SetupForm {
    name: String,
    #[serde(default)]
    phone_number: String,
    league_name: String,
    #[serde(default)]
    default_language: String,
    #[serde(default)]
    signal_group_id: String,
    #[serde(default)]
    signal_from_number: String,
    #[serde(default)]
    rss_feed_url: String,
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
    let league_name = form.league_name.trim().to_string();
    if league_name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Liga-Name darf nicht leer sein."));
    }
    if league_name.len() > 255 {
        return Err((StatusCode::BAD_REQUEST, "Liga-Name zu lang."));
    }
    let lang = form.default_language.trim();
    let lang = if lang.is_empty() { "de" } else { lang };
    if !VALID_LOCALES.contains(&lang) {
        return Err((StatusCode::BAD_REQUEST, "Unbekannte Sprache."));
    }

    let phone = form.phone_number.trim().to_string();
    let phone_opt: Option<&str> = if phone.is_empty() { None } else { Some(&phone) };

    // Create the league first — the user row needs its id as a foreign key.
    let league_id = state
        .repos
        .leagues
        .create(&league_name)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    // Persist initial league settings. Empty inputs are *not* written, so
    // `LeagueConfig::default()` keeps applying for unset keys.
    persist_setting(&state, league_id, LeagueConfig::KEY_DEFAULT_LANGUAGE, lang).await?;
    persist_setting(
        &state,
        league_id,
        LeagueConfig::KEY_SIGNAL_GROUP_ID,
        form.signal_group_id.trim(),
    )
    .await?;
    persist_setting(
        &state,
        league_id,
        LeagueConfig::KEY_SIGNAL_FROM_NUMBER,
        form.signal_from_number.trim(),
    )
    .await?;
    persist_setting(
        &state,
        league_id,
        LeagueConfig::KEY_RSS_FEED_URL,
        form.rss_feed_url.trim(),
    )
    .await?;

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
            league_id,
            language: lang,
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
        lang_code: lang_static(lang),
        magic_link,
        name,
    };
    Ok((updated_jar, Html(page.render().unwrap()).into_response()))
}

async fn persist_setting(
    state: &AppState,
    league_id: Uuid,
    key: &str,
    value: &str,
) -> Result<(), (StatusCode, &'static str)> {
    let v: Option<&str> = if value.is_empty() { None } else { Some(value) };
    state
        .repos
        .leagues
        .set_setting(league_id, key, v)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))
}

fn lang_static(lang: &str) -> &'static str {
    match lang {
        "en" => "en",
        "es" => "es",
        "fr" => "fr",
        _ => "de",
    }
}
