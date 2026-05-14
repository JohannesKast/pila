//! League (Tipp-Liga) management routes.
//!
//! Three actor levels touch these routes:
//!   - Regular user: nothing here is exposed.
//!   - League admin (`is_admin`): no access; a league admin only manages
//!     their own league's users via `admin.rs`.
//!   - Super-admin (`can_create_league`): can list every league, create
//!     new ones, and edit per-league settings (Signal config, default
//!     language, RSS feed, ...).

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, Redirect},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::SuperAdminUser;
use crate::handlers::util::{render_template, t_for};
use crate::repo::league::{League, LeagueConfig};
use crate::translations::T;
use crate::AppState;

// ─── List + create form ──────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/leagues.html")]
struct LeaguesTemplate {
    leagues: Vec<LeagueRow>,
    t: T,
    lang_code: String,
}

struct LeagueRow {
    id: Uuid,
    name: String,
    config: LeagueConfig,
}

pub async fn leagues_list(
    State(state): State<AppState>,
    SuperAdminUser(user): SuperAdminUser,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    let leagues = state
        .repos
        .leagues
        .list()
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let mut rows = Vec::with_capacity(leagues.len());
    for l in leagues {
        let cfg = state
            .repos
            .leagues
            .get_config(l.id)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
        rows.push(LeagueRow {
            id: l.id,
            name: l.name,
            config: cfg,
        });
    }

    let template = LeaguesTemplate {
        leagues: rows,
        t: t_for(&state, &user.language),
        lang_code: user.language.clone(),
    };
    render_template(&template)
}

#[derive(Template)]
#[template(path = "admin/leagues_new.html")]
struct LeaguesNewTemplate {
    t: T,
    lang_code: String,
}

pub async fn leagues_new_form(
    State(state): State<AppState>,
    SuperAdminUser(user): SuperAdminUser,
) -> Html<String> {
    let template = LeaguesNewTemplate {
        t: t_for(&state, &user.language),
        lang_code: user.language.clone(),
    };
    render_template(&template).unwrap_or_else(|_| Html("Interner Fehler".to_string()))
}

#[derive(Deserialize)]
pub struct LeagueCreateForm {
    pub name: String,
}

pub async fn leagues_create(
    State(state): State<AppState>,
    SuperAdminUser(_user): SuperAdminUser,
    Form(form): Form<LeagueCreateForm>,
) -> Result<Redirect, (StatusCode, &'static str)> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Liga-Name darf nicht leer sein."));
    }
    if name.len() > 255 {
        return Err((StatusCode::BAD_REQUEST, "Liga-Name zu lang."));
    }
    state
        .repos
        .leagues
        .create(name)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
    Ok(Redirect::to("/admin/leagues"))
}

// ─── Per-league settings form ────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/league_settings.html")]
struct LeagueSettingsTemplate {
    league: League,
    config: LeagueConfig,
    t: T,
    lang_code: String,
}

pub async fn league_settings_form(
    State(state): State<AppState>,
    SuperAdminUser(user): SuperAdminUser,
    Path(league_id): Path<Uuid>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    let league = state
        .repos
        .leagues
        .find_by_id(league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::NOT_FOUND, "Liga nicht gefunden."))?;
    let config = state
        .repos
        .leagues
        .get_config(league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
    let template = LeagueSettingsTemplate {
        league,
        config,
        t: t_for(&state, &user.language),
        lang_code: user.language.clone(),
    };
    render_template(&template)
}

#[derive(Deserialize)]
pub struct LeagueSettingsForm {
    #[serde(default)]
    pub signal_group_id: String,
    #[serde(default)]
    pub signal_from_number: String,
    #[serde(default)]
    pub default_language: String,
    #[serde(default)]
    pub rss_feed_url: String,
    #[serde(default)]
    pub ko_only: bool,
}

const VALID_LOCALES: &[&str] = &["de", "en", "es", "fr"];

pub async fn league_settings_save(
    State(state): State<AppState>,
    SuperAdminUser(_user): SuperAdminUser,
    Path(league_id): Path<Uuid>,
    Form(form): Form<LeagueSettingsForm>,
) -> Result<Redirect, (StatusCode, &'static str)> {
    // Validate first; reject the whole submission rather than persist a
    // partial update. Empty string = clear the setting (None).
    let lang = form.default_language.trim();
    if !lang.is_empty() && !VALID_LOCALES.contains(&lang) {
        return Err((StatusCode::BAD_REQUEST, "Unbekannte Sprache."));
    }

    set_or_clear_bool(&state, league_id, LeagueConfig::KEY_KO_ONLY, form.ko_only).await?;
    set_or_clear(&state, league_id, LeagueConfig::KEY_SIGNAL_GROUP_ID, &form.signal_group_id).await?;
    set_or_clear(&state, league_id, LeagueConfig::KEY_SIGNAL_FROM_NUMBER, &form.signal_from_number).await?;
    set_or_clear(&state, league_id, LeagueConfig::KEY_DEFAULT_LANGUAGE, lang).await?;
    set_or_clear(&state, league_id, LeagueConfig::KEY_RSS_FEED_URL, &form.rss_feed_url).await?;

    Ok(Redirect::to("/admin/leagues"))
}

async fn set_or_clear(
    state: &AppState,
    league_id: Uuid,
    key: &str,
    value: &str,
) -> Result<(), (StatusCode, &'static str)> {
    let trimmed = value.trim();
    let v: Option<&str> = if trimmed.is_empty() { None } else { Some(trimmed) };
    state
        .repos
        .leagues
        .set_setting(league_id, key, v)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))
}

async fn set_or_clear_bool(
    state: &AppState,
    league_id: Uuid,
    key: &str,
    value: bool,
) -> Result<(), (StatusCode, &'static str)> {
    let v: Option<&str> = if value { Some("true") } else { None };
    state
        .repos
        .leagues
        .set_setting(league_id, key, v)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))
}
