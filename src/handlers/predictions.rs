// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Per-match score tip and the singleton Weltmeister pick.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, Redirect},
};
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::util::{flag_url, render_template, t_err, HandlerError};
use crate::scoring::{self, MatchScoringSystem};
use crate::stage::Stage;
use crate::translations::T;
use crate::AppState;

#[derive(Template)]
#[template(path = "predict_form.html")]
struct PredictFormTemplate {
    match_id: i32,
    score_home: i32,
    score_away: i32,
    winner_only_mode: bool,
    allow_draw_prediction: bool,
    home_name: String,
    away_name: String,
    home_flag: String,
    away_flag: String,
}

#[derive(Deserialize)]
pub struct PredictionForm {
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    #[serde(default)]
    pub outcome: String,
}

pub async fn predict_match(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(match_id): Path<i32>,
    Form(form): Form<PredictionForm>,
) -> Result<Html<String>, HandlerError> {
    let lang = &user.language;
    let t = crate::handlers::util::t_for(&state, lang);

    let m = state
        .repos
        .matches
        .find_lock_info(match_id)
        .await
        .map_err(|_| {
            t_err(
                &state,
                lang,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })?
        .ok_or_else(|| t_err(&state, lang, StatusCode::NOT_FOUND, "error-match-not-found"))?;

    let config = state
        .repos
        .leagues
        .get_config(user.league_id)
        .await
        .map_err(|_| {
            t_err(
                &state,
                lang,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })?;

    let now = crate::time::now(&state.mock_now);

    if m.team_home_id.is_none() || m.team_away_id.is_none() {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-match-not-fixed",
        ));
    }

    if config.predict_knockout_only && m.stage == Stage::Group {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-group-stage-disabled",
        ));
    }

    if let Some(start_time) = m.kickoff_time {
        if start_time < now {
            return Err(t_err(
                &state,
                lang,
                StatusCode::BAD_REQUEST,
                "error-match-locked",
            ));
        }
    }

    let allow_draw_prediction = m.stage == Stage::Group;
    let (score_home, score_away) = normalize_prediction(&t, &config, &form, allow_draw_prediction)?;

    state
        .repos
        .predictions
        .upsert(user.id, match_id, score_home, score_away)
        .await
        .map_err(|_| {
            t_err(
                &state,
                lang,
                StatusCode::INTERNAL_SERVER_ERROR,
                "error-database",
            )
        })?;

    let template = PredictFormTemplate {
        match_id,
        score_home,
        score_away,
        winner_only_mode: config.match_scoring_system == MatchScoringSystem::WinnerOnly,
        allow_draw_prediction,
        home_name: m.home_name,
        away_name: m.away_name,
        home_flag: flag_url(&m.home_flag_code),
        away_flag: flag_url(&m.away_flag_code),
    };
    render_template(&template)
}

fn normalize_prediction(
    t: &T,
    config: &crate::repo::league::LeagueConfig,
    form: &PredictionForm,
    allow_draw_prediction: bool,
) -> Result<(i32, i32), HandlerError> {
    if config.match_scoring_system == MatchScoringSystem::WinnerOnly {
        let outcome = scoring::outcome_bet_from_form(form.outcome.trim(), allow_draw_prediction)
            .ok_or_else(|| (StatusCode::BAD_REQUEST, t.get("error-prediction-outcome")))?;
        return Ok(scoring::outcome_bet_to_stored_scores(outcome));
    }

    let score_home = form.score_home.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            t.get("error-prediction-home-missing"),
        )
    })?;
    let score_away = form.score_away.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            t.get("error-prediction-away-missing"),
        )
    })?;

    if !(0..=20).contains(&score_home) || !(0..=20).contains(&score_away) {
        return Err((
            StatusCode::BAD_REQUEST,
            t.get("error-prediction-out-of-range"),
        ));
    }

    Ok((score_home, score_away))
}

fn deserialize_optional_int<'de, D>(de: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(de)?;
    match s.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => s.parse().map(Some).map_err(serde::de::Error::custom),
    }
}

#[derive(Deserialize)]
pub struct SpecialPredictionForm {
    #[serde(default, deserialize_with = "deserialize_optional_int")]
    pub champion_id: Option<i32>,
}

pub async fn predict_special(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Form(form): Form<SpecialPredictionForm>,
) -> Result<Redirect, HandlerError> {
    let lang = &user.language;
    let now = crate::time::now(&state.mock_now);

    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let config = state
        .repos
        .leagues
        .get_config(user.league_id)
        .await
        .map_err(|_| db_err())?;

    let first_kickoff = if config.predict_knockout_only {
        state
            .repos
            .matches
            .first_knockout_kickoff()
            .await
            .map_err(|_| db_err())?
    } else {
        state
            .repos
            .matches
            .first_kickoff()
            .await
            .map_err(|_| db_err())?
    };

    if first_kickoff.is_some_and(|dt| dt < now) {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-champion-locked",
        ));
    }

    if let Some(cid) = form.champion_id {
        let exists = state
            .repos
            .teams
            .exists_real(cid)
            .await
            .map_err(|_| db_err())?;
        if !exists {
            return Err(t_err(
                &state,
                lang,
                StatusCode::BAD_REQUEST,
                "error-unknown-team",
            ));
        }
    }

    state
        .repos
        .special_predictions
        .upsert(user.id, form.champion_id)
        .await
        .map_err(|_| db_err())?;

    Ok(Redirect::to("/"))
}
