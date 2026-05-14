//! Per-match score tip and the singleton Weltmeister pick.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, Redirect},
};
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::util::{flag_url, render_template};
use crate::scoring::{self, MatchScoringSystem};
use crate::stage::Stage;
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
) -> Result<Html<String>, (StatusCode, &'static str)> {
    let m = state
        .repos
        .matches
        .find_lock_info(match_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::NOT_FOUND, "Match not found"))?;

    let config = state
        .repos
        .leagues
        .get_config(user.league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let now = crate::time::now(&state.mock_now);

    if m.team_home_id.is_none() || m.team_away_id.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Begegnung steht noch nicht fest. Tipp nicht möglich.",
        ));
    }

    if config.predict_knockout_only && m.stage == Stage::Group {
        return Err((
            StatusCode::BAD_REQUEST,
            "In dieser Liga werden Gruppenspiele nicht getippt.",
        ));
    }

    if let Some(start_time) = m.kickoff_time {
        if start_time < now {
            return Err((
                StatusCode::BAD_REQUEST,
                "Das Spiel hat bereits begonnen. Tipps sind gesperrt.",
            ));
        }
    }

    let allow_draw_prediction = m.stage == Stage::Group;
    let (score_home, score_away) = normalize_prediction(&config, &form, allow_draw_prediction)?;

    state
        .repos
        .predictions
        .upsert(user.id, match_id, score_home, score_away)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

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
    config: &crate::repo::league::LeagueConfig,
    form: &PredictionForm,
    allow_draw_prediction: bool,
) -> Result<(i32, i32), (StatusCode, &'static str)> {
    if config.match_scoring_system == MatchScoringSystem::WinnerOnly {
        let outcome = scoring::outcome_bet_from_form(form.outcome.trim(), allow_draw_prediction)
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Ungültiger Tipp: Sieger-Auswahl nicht erkannt.",
            ))?;
        return Ok(scoring::outcome_bet_to_stored_scores(outcome));
    }

    let score_home = form
        .score_home
        .ok_or((StatusCode::BAD_REQUEST, "Ungültiger Tipp: Heimtore fehlen."))?;
    let score_away = form.score_away.ok_or((
        StatusCode::BAD_REQUEST,
        "Ungültiger Tipp: Auswärtstore fehlen.",
    ))?;

    if !(0..=20).contains(&score_home) || !(0..=20).contains(&score_away) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Ungültiger Tipp: Werte müssen zwischen 0 und 20 liegen.",
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
) -> Result<Redirect, (StatusCode, &'static str)> {
    let now = crate::time::now(&state.mock_now);

    let config = state
        .repos
        .leagues
        .get_config(user.league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let first_kickoff = if config.predict_knockout_only {
        state
            .repos
            .matches
            .first_knockout_kickoff()
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
    } else {
        state
            .repos
            .matches
            .first_kickoff()
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
    };

    if first_kickoff.is_some_and(|dt| dt < now) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Turnier hat begonnen. Weltmeister-Tipp ist gesperrt.",
        ));
    }

    if let Some(cid) = form.champion_id {
        let exists = state
            .repos
            .teams
            .exists_real(cid)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
        if !exists {
            return Err((StatusCode::BAD_REQUEST, "Unbekanntes Team."));
        }
    }

    state
        .repos
        .special_predictions
        .upsert(user.id, form.champion_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    Ok(Redirect::to("/"))
}
