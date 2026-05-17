// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

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
use crate::handlers::util::{render_template, t_err, t_for, HandlerError};
use crate::repo::league::{League, LeagueConfig};
use crate::scoring::MatchScoringSystem;
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
) -> Result<Html<String>, HandlerError> {
    let lang = &user.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let leagues = state.repos.leagues.list().await.map_err(|_| db_err())?;

    let mut rows = Vec::with_capacity(leagues.len());
    for l in leagues {
        let cfg = state
            .repos
            .leagues
            .get_config(l.id)
            .await
            .map_err(|_| db_err())?;
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
    render_template(&template).unwrap_or_else(|_| Html("Internal error".to_string()))
}

#[derive(Deserialize)]
pub struct LeagueCreateForm {
    pub name: String,
}

pub async fn leagues_create(
    State(state): State<AppState>,
    SuperAdminUser(user): SuperAdminUser,
    Form(form): Form<LeagueCreateForm>,
) -> Result<Redirect, HandlerError> {
    let lang = &user.language;
    let name = form.name.trim();
    if name.is_empty() {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-league-name-empty",
        ));
    }
    if name.len() > 255 {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-league-name-too-long",
        ));
    }
    state.repos.leagues.create(name).await.map_err(|_| {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    })?;
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
) -> Result<Html<String>, HandlerError> {
    let lang = &user.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let league = state
        .repos
        .leagues
        .find_by_id(league_id)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(|| {
            t_err(
                &state,
                lang,
                StatusCode::NOT_FOUND,
                "error-league-not-found",
            )
        })?;
    let config = state
        .repos
        .leagues
        .get_config(league_id)
        .await
        .map_err(|_| db_err())?;
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
    #[serde(default)]
    pub match_scoring_system: String,
}

const VALID_LOCALES: &[&str] = &["de", "en", "es", "fr"];

pub async fn league_settings_save(
    State(state): State<AppState>,
    SuperAdminUser(user): SuperAdminUser,
    Path(league_id): Path<Uuid>,
    Form(form): Form<LeagueSettingsForm>,
) -> Result<Redirect, HandlerError> {
    let user_lang = &user.language;
    let err = |status: StatusCode, key: &str| t_err(&state, user_lang, status, key);

    // Validate first; reject the whole submission rather than persist a
    // partial update. Empty string = clear the setting (None).
    let lang = form.default_language.trim();
    if !lang.is_empty() && !VALID_LOCALES.contains(&lang) {
        return Err(err(StatusCode::BAD_REQUEST, "error-unknown-language"));
    }
    let scoring_system = MatchScoringSystem::from_setting_value(form.match_scoring_system.trim())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "error-unknown-scoring"))?;

    set_or_clear_bool(
        &state,
        user_lang,
        league_id,
        LeagueConfig::KEY_KO_ONLY,
        form.ko_only,
    )
    .await?;
    set_or_clear(
        &state,
        user_lang,
        league_id,
        LeagueConfig::KEY_SIGNAL_GROUP_ID,
        &form.signal_group_id,
    )
    .await?;
    set_or_clear(
        &state,
        user_lang,
        league_id,
        LeagueConfig::KEY_SIGNAL_FROM_NUMBER,
        &form.signal_from_number,
    )
    .await?;
    set_or_clear(
        &state,
        user_lang,
        league_id,
        LeagueConfig::KEY_DEFAULT_LANGUAGE,
        lang,
    )
    .await?;
    set_or_clear(
        &state,
        user_lang,
        league_id,
        LeagueConfig::KEY_RSS_FEED_URL,
        &form.rss_feed_url,
    )
    .await?;
    set_or_clear(
        &state,
        user_lang,
        league_id,
        LeagueConfig::KEY_MATCH_SCORING_SYSTEM,
        scoring_system.as_setting_value(),
    )
    .await?;

    Ok(Redirect::to("/admin/leagues"))
}

async fn set_or_clear(
    state: &AppState,
    user_lang: &str,
    league_id: Uuid,
    key: &str,
    value: &str,
) -> Result<(), HandlerError> {
    let trimmed = value.trim();
    let v: Option<&str> = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    state
        .repos
        .leagues
        .set_setting(league_id, key, v)
        .await
        .map_err(|_| {
            t_err(
                state,
                user_lang,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })
}

async fn set_or_clear_bool(
    state: &AppState,
    user_lang: &str,
    league_id: Uuid,
    key: &str,
    value: bool,
) -> Result<(), HandlerError> {
    let v: Option<&str> = if value { Some("true") } else { None };
    state
        .repos
        .leagues
        .set_setting(league_id, key, v)
        .await
        .map_err(|_| {
            t_err(
                state,
                user_lang,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })
}
