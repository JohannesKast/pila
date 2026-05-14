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
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::handlers::util::{build_magic_link, make_login_cookie, render_template};
use crate::notifier::{self, signal_configured};
use crate::repo::league::LeagueConfig;
use crate::repo::{Repos};
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
    Ok(render_template(&template)?.into_response())
}

const VALID_LOCALES: &[&str] = &["de", "en", "es", "fr"];

#[derive(Deserialize)]
pub struct SetupForm {
    name: String,
    #[serde(default)]
    phone_number: String,
    #[serde(default)]
    email: String,
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
    let email = form.email.trim().to_string();
    let email_opt: Option<&str> = if email.is_empty() { None } else { Some(&email) };

    let id = Uuid::new_v4();
    let token = Uuid::new_v4().to_string();
    let magic_link = build_magic_link(&token, &state.base_url);

    // ── Transaction: league + settings + user + can_create_league ──
    // If any step fails the whole setup rolls back; a half-created league
    // or orphan user can never exist.
    {
        let mut tx = state
            .db
            .as_ref()
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "DB not available"))?
            .begin()
            .await
            .map_err(|e| {
                tracing::error!(%e, "setup: begin transaction failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error")
            })?;

        // 1. Create league
        let league_id: Uuid = sqlx::query_scalar!(
            "INSERT INTO leagues (name) VALUES ($1) RETURNING id",
            &league_name
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(%e, "setup: create league failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error")
        })?;

        // 2. Persist settings (only non-empty values)
        let settings: &[(&str, &str)] = &[
            (LeagueConfig::KEY_DEFAULT_LANGUAGE, lang),
            (LeagueConfig::KEY_SIGNAL_GROUP_ID, form.signal_group_id.trim()),
            (LeagueConfig::KEY_SIGNAL_FROM_NUMBER, form.signal_from_number.trim()),
            (LeagueConfig::KEY_RSS_FEED_URL, form.rss_feed_url.trim()),
        ];
        for (key, value) in settings {
            if value.is_empty() {
                continue;
            }
            sqlx::query!(
                "INSERT INTO league_settings (league_id, key, value) VALUES ($1, $2, $3) \
                 ON CONFLICT (league_id, key) DO UPDATE SET value = EXCLUDED.value",
                league_id,
                key,
                value
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                tracing::error!(%e, key, "setup: insert setting failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error")
            })?;
        }

        // 3. Create user
        sqlx::query!(
            "INSERT INTO users (id, name, token, is_admin, phone_number, email, league_id, language) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            id,
            &name,
            &token,
            true, // is_admin
            phone_opt,
            email_opt,
            league_id,
            lang
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!(%e, "setup: create user failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error")
        })?;

        // 4. Grant super-admin
        sqlx::query!("UPDATE users SET can_create_league = TRUE WHERE id = $1", id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                tracing::error!(%e, "setup: grant can_create_league failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error")
            })?;

        tx.commit().await.map_err(|e| {
            tracing::error!(%e, "setup: commit failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error")
        })?;
    }

    // Signal invite is outside the transaction — best-effort side effect.
    if let Some(p) = phone_opt {
        if signal_configured(&state.signal_api_url, &state.signal_from_number) {
            if let Err(e) = notifier::send_invite_via_signal(p, &name, &magic_link, &state.signal_api_url, &state.signal_from_number).await {
                tracing::warn!("Setup: Signal-Einladung an {p} fehlgeschlagen: {e}");
            }
        }
    }
    // Email invite — also best-effort.
    if let Some(e) = email_opt {
        if let Some(ref smtp) = state.smtp_config {
            if let Err(err) = crate::mail::send_invite_email(smtp, &name, e, &magic_link).await {
                tracing::warn!("Setup: E-Mail-Einladung an {e} fehlgeschlagen: {err}");
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
