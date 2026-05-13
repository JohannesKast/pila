//! Per-match score tip and the singleton Weltmeister pick.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, Redirect},
};
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::util::render_template;
use crate::AppState;

#[derive(Template)]
#[template(path = "predict_form.html")]
struct PredictFormTemplate {
    match_id: i32,
    score_home: i32,
    score_away: i32,
}

#[derive(Deserialize)]
pub struct PredictionForm {
    pub score_home: i32,
    pub score_away: i32,
}

pub async fn predict_match(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(match_id): Path<i32>,
    Form(form): Form<PredictionForm>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    if !(0..=20).contains(&form.score_home) || !(0..=20).contains(&form.score_away) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Ungültiger Tipp: Werte müssen zwischen 0 und 20 liegen.",
        ));
    }

    let m = state
        .repos
        .matches
        .find_lock_info(match_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?
        .ok_or((StatusCode::NOT_FOUND, "Match not found"))?;

    let now = chrono::Utc::now();

    if m.team_home_id.is_none() || m.team_away_id.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Begegnung steht noch nicht fest. Tipp nicht möglich.",
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

    state
        .repos
        .predictions
        .upsert(user.id, match_id, form.score_home, form.score_away)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let template = PredictFormTemplate {
        match_id,
        score_home: form.score_home,
        score_away: form.score_away,
    };
    Ok(render_template(&template)?)
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
    let now = chrono::Utc::now();

    let first_kickoff = state
        .repos
        .matches
        .first_kickoff()
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

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
