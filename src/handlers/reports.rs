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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{
        League, MatchdayReport, MatchdayReportRepo, MemoryBootstrapRepo, MemoryInviteRepo,
        MemoryLeagueRepo, MemoryMatchRepo, MemoryMatchdayReportRepo, MemoryNotificationRepo,
        MemoryPredictionRepo, MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo,
        MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    struct Harness {
        state: AppState,
        reports: Arc<MemoryMatchdayReportRepo>,
        user: AuthenticatedUser,
    }

    fn harness() -> Harness {
        let reports = Arc::new(MemoryMatchdayReportRepo::new());
        let leagues = Arc::new(MemoryLeagueRepo::new());
        leagues.seed(League {
            id: DEFAULT_LEAGUE_ID,
            name: "Default".into(),
            notifications_bootstrapped: true,
        });
        let repos = Repos {
            bootstrap: Arc::new(MemoryBootstrapRepo::new()),
            users: Arc::new(MemoryUserRepo::new()),
            leagues,
            matches: Arc::new(MemoryMatchRepo::new()),
            predictions: Arc::new(MemoryPredictionRepo::new()),
            special_predictions: Arc::new(MemorySpecialPredictionRepo::new()),
            teams: Arc::new(MemoryTeamRepo::new()),
            settings: Arc::new(MemorySettingsRepo::new()),
            invites: Arc::new(MemoryInviteRepo::new()),
            notifications: Arc::new(MemoryNotificationRepo::new()),
            reports: reports.clone(),
        };
        let state = AppState {
            jerseys: crate::jersey::load(),
            news: crate::news::NewsCache::from_env(),
            repos,
            translations: crate::translations::load_all(),
            concurrency_limit: Arc::new(tokio::sync::Semaphore::new(100)),
            base_url: "http://localhost:8000".into(),
            signal_api_url: None,
            signal_from_number: None,
            signal_group_id: None,
            http_client: reqwest::Client::new(),
            smtp_config: None,
            mock_now: crate::time::new_mock_time(),
            dev_mode: false,
        };
        let user = AuthenticatedUser {
            id: Uuid::new_v4(),
            name: "Alice".into(),
            real_name: "Alice".into(),
            is_admin: false,
            can_create_league: false,
            phone_number: None,
            email: None,
            jersey_preset: "classic".into(),
            language: "en".into(),
            league_id: DEFAULT_LEAGUE_ID,
        };
        Harness {
            state,
            reports,
            user,
        }
    }

    fn report(date: NaiveDate, content: &str) -> MatchdayReport {
        MatchdayReport {
            league_id: DEFAULT_LEAGUE_ID,
            matchday_date: date,
            language: "en".into(),
            content: content.into(),
            model: "gemini::gemini-2.5-flash".into(),
            generated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn matchday_report_renders_requested_partial() {
        let h = harness();
        let older = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let requested = NaiveDate::from_ymd_opt(2026, 6, 11).unwrap();
        h.reports.insert(&report(older, "## Older")).await.unwrap();
        h.reports
            .insert(&report(requested, "## Requested\n\n**recap**"))
            .await
            .unwrap();

        let html = matchday_report(
            State(h.state),
            h.user,
            Query(ReportQuery {
                date: Some(requested.to_string()),
            }),
        )
        .await
        .0;

        assert!(html.contains("id=\"matchday-report\""));
        assert!(html.contains("<h2>Requested</h2>"));
        assert!(html.contains("<strong>recap</strong>"));
        assert!(html.contains(&format!("/reports?date={older}")));
    }

    #[tokio::test]
    async fn matchday_report_renders_empty_when_no_report_exists() {
        let h = harness();
        let html = matchday_report(State(h.state), h.user, Query(ReportQuery { date: None }))
            .await
            .0;
        assert!(!html.contains("id=\"matchday-report\""));
    }
}
