// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! AI matchday recap navigation (`GET /reports`).
//!
//! Returns the recap card partial for a given matchday (or the latest when no
//! `date` is supplied). The dashboard renders the same partial inline; the
//! arrow buttons in the card `hx-get` this route to swap in a neighbouring
//! matchday's recap.

use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::handlers::services::build_matchday_report_view;
use crate::handlers::util::render_template;
use crate::translations::T;
use crate::views::MatchdayReportView;
use crate::AppState;

#[derive(Template)]
#[template(path = "partials/matchday_report.html")]
struct ReportPanelTemplate {
    report: Option<MatchdayReportView>,
    t: T,
}

#[derive(Debug, Deserialize)]
pub struct ReportQuery {
    /// ISO date (`YYYY-MM-DD`) of the matchday to show. Absent → latest.
    date: Option<String>,
}

pub async fn matchday_report(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(q): Query<ReportQuery>,
) -> Html<String> {
    let date = q
        .date
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let report = build_matchday_report_view(&state.repos, user.league_id, date).await;
    let t = crate::handlers::util::t_for(&state, &user.language);
    render_template(&ReportPanelTemplate { report, t })
        .unwrap_or_else(|_| Html("Internal error".to_string()))
}
