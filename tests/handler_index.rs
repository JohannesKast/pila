//! Regression test for the blank-dashboard bug caused by a `<head>` script
//! referencing `document.body` before the `<body>` tag is parsed.  The
//! thrown error blocked *all* inline scripts, so the tab-panels stayed
//! `hidden` and the HTMX CSRF-token listener was never registered.

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Query, State};
use uuid::Uuid;

use pila::auth::MaybeAuthenticatedUser;
use pila::handlers::index;
use pila::handlers::jersey::{jersey_post, JerseyPostQuery};
use pila::handlers::util::league_scope_path;
use pila::repo::fixture::FakeMatch;
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::prediction::FakeFinishedRow;
use pila::repo::user::UserFull;
use pila::repo::{
    MemoryBootstrapRepo, MemoryInviteRepo, MemoryMatchRepo, MemoryNotificationRepo,
    MemoryPredictionRepo, MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo,
    MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
};
use pila::stage::Stage;
use pila::AppState;

struct Harness {
    state: AppState,
    users: Arc<MemoryUserRepo>,
}

fn build_harness() -> Harness {
    let users = Arc::new(MemoryUserRepo::new());
    let leagues = Arc::new(MemoryLeagueRepo::new());
    leagues.seed(League {
        id: DEFAULT_LEAGUE_ID,
        name: "Default".into(),
        notifications_bootstrapped: true,
    });

    let repos = Repos {
        bootstrap: Arc::new(MemoryBootstrapRepo::new()),
        users: users.clone(),
        leagues,
        matches: Arc::new(MemoryMatchRepo::new()),
        predictions: Arc::new(MemoryPredictionRepo::new()),
        special_predictions: Arc::new(MemorySpecialPredictionRepo::new()),
        teams: Arc::new(MemoryTeamRepo::new()),
        settings: Arc::new(MemorySettingsRepo::new()),
        invites: Arc::new(MemoryInviteRepo::new()),
        notifications: Arc::new(MemoryNotificationRepo::new()),
    };

    let state = AppState {
        jerseys: pila::jersey::load(),
        news: pila::news::NewsCache::from_env(),
        repos,
        translations: pila::translations::load_all(),
        concurrency_limit: Arc::new(tokio::sync::Semaphore::new(100)),
        base_url: "http://localhost:8000".into(),
        signal_api_url: None,
        signal_from_number: None,
        signal_group_id: None,
        http_client: reqwest::Client::new(),
        smtp_config: None,
        mock_now: pila::time::new_mock_time(),
        dev_mode: false,
    };

    Harness { state, users }
}

fn fake_user() -> pila::auth::AuthenticatedUser {
    pila::auth::AuthenticatedUser {
        id: Uuid::new_v4(),
        name: "Tester".into(),
        real_name: "Tester".into(),
        is_admin: false,
        can_create_league: false,
        phone_number: None,
        email: None,
        jersey_preset: "classic".into(),
        language: "de".into(),
        league_id: DEFAULT_LEAGUE_ID,
    }
}

#[tokio::test]
async fn index_renders_with_valid_csp_and_without_document_body_in_head() {
    let h = build_harness();
    let user = fake_user();

    // Seed the user so the repo can resolve it during the request.
    h.users.seed(
        UserFull {
            id: user.id,
            name: user.name.clone(),
            real_name: user.name.clone(),
            token: "test-token".into(),
            phone_number: None,
            email: None,
            is_admin: false,
            can_create_league: false,
            league_id: user.league_id,
            language: "de".into(),
        },
        "classic",
    );

    let response = index(State(h.state), MaybeAuthenticatedUser(Some(user)))
        .await
        .expect("index handler should succeed");

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let html = String::from_utf8(bytes.to_vec()).expect("body should be valid UTF-8");

    // Sanity: the page actually rendered tab content.
    assert!(
        html.contains(r#"id="tab-container""#),
        "index page must render tab container"
    );
    let scope_path = league_scope_path(DEFAULT_LEAGUE_ID);
    assert!(
        html.contains(&format!(r#"hx-get="{scope_path}/profile/jersey-picker""#)),
        "profile controls must stay within the league scope"
    );

    // Regression guard: <head> scripts must not touch document.body
    // before the <body> tag is parsed.
    let head_end = html
        .find("</head>")
        .expect("valid HTML must contain </head>");
    let head = &html[..head_end];
    assert!(
        !head.contains("document.body.addEventListener"),
        "head script must not reference document.body before body is parsed"
    );

    // CSP must allow inline scripts (otherwise all JS on the page breaks)
    assert!(
        head.contains("script-src 'self' 'unsafe-inline'"),
        "CSP must include 'unsafe-inline' in script-src to allow inline scripts"
    );
}

/// Build a harness exposing the match and prediction repos so a test can seed
/// fixtures and tips directly.
struct FullHarness {
    state: AppState,
    users: Arc<MemoryUserRepo>,
    matches: Arc<MemoryMatchRepo>,
    predictions: Arc<MemoryPredictionRepo>,
}

fn build_full_harness() -> FullHarness {
    let users = Arc::new(MemoryUserRepo::new());
    let matches = Arc::new(MemoryMatchRepo::new());
    let predictions = Arc::new(MemoryPredictionRepo::new());
    let leagues = Arc::new(MemoryLeagueRepo::new());
    leagues.seed(League {
        id: DEFAULT_LEAGUE_ID,
        name: "Default".into(),
        notifications_bootstrapped: true,
    });

    let repos = Repos {
        bootstrap: Arc::new(MemoryBootstrapRepo::new()),
        users: users.clone(),
        leagues,
        matches: matches.clone(),
        predictions: predictions.clone(),
        special_predictions: Arc::new(MemorySpecialPredictionRepo::new()),
        teams: Arc::new(MemoryTeamRepo::new()),
        settings: Arc::new(MemorySettingsRepo::new()),
        invites: Arc::new(MemoryInviteRepo::new()),
        notifications: Arc::new(MemoryNotificationRepo::new()),
    };

    let state = AppState {
        jerseys: pila::jersey::load(),
        news: pila::news::NewsCache::from_env(),
        repos,
        translations: pila::translations::load_all(),
        concurrency_limit: Arc::new(tokio::sync::Semaphore::new(100)),
        base_url: "http://localhost:8000".into(),
        signal_api_url: None,
        signal_from_number: None,
        signal_group_id: None,
        http_client: reqwest::Client::new(),
        smtp_config: None,
        mock_now: pila::time::new_mock_time(),
        dev_mode: false,
    };

    FullHarness {
        state,
        users,
        matches,
        predictions,
    }
}

/// A finished match the current user nailed exactly — as the only tipper — must
/// render the per-game "solo" marker on the card and a badge chip in the table.
#[tokio::test]
async fn index_renders_badge_markers_for_exact_solo_hit() {
    let h = build_full_harness();
    let user = fake_user();
    h.users.seed(
        UserFull {
            id: user.id,
            name: user.name.clone(),
            real_name: user.name.clone(),
            token: "test-token".into(),
            phone_number: None,
            email: None,
            is_admin: false,
            can_create_league: false,
            league_id: user.league_id,
            language: "de".into(),
        },
        "classic",
    );

    let kickoff = chrono::Utc::now() - chrono::Duration::hours(3);

    // Seed a finished group match and the user's exact 2:1 tip.
    let mut m = FakeMatch::locked_unfinished(1, kickoff);
    m.status = "finished".into();
    m.score_home = Some(2);
    m.score_away = Some(1);
    h.matches.seed(m);
    h.matches.record_prediction(user.id, 1, 2, 1);

    // The same prediction feeds the aggregate badges (drives the table chip).
    h.predictions.seed_finished(FakeFinishedRow {
        user_id: user.id,
        league_id: user.league_id,
        match_id: 1,
        stage: Stage::Group,
        kickoff,
        score_home: 2,
        score_away: 1,
        predicted_home: 2,
        predicted_away: 1,
    });

    let response = index(State(h.state), MaybeAuthenticatedUser(Some(user)))
        .await
        .expect("index handler should succeed");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(bytes.to_vec()).unwrap();

    // Per-game marker: a lone exact hit is a solo (💎).
    assert!(
        html.contains("💎"),
        "finished match card must show the solo-hit marker"
    );
    // Table chip: the exact tip earns at least the exact-count achievement,
    // rendered as a count chip with a hover tooltip.
    assert!(
        html.contains("cursor:help"),
        "leaderboard row must render at least one badge chip"
    );
    assert!(
        html.contains("🎯"),
        "exact tip must surface the exact-count badge chip"
    );
}

/// The jersey OOB refresh re-renders the user's leaderboard row, so it must
/// carry the badge chips too — otherwise changing a jersey would blank them
/// until the next full page load.
#[tokio::test]
async fn jersey_change_oob_row_keeps_badge_chips() {
    let h = build_full_harness();
    let user = fake_user();
    h.users.seed(
        UserFull {
            id: user.id,
            name: user.name.clone(),
            real_name: user.name.clone(),
            token: "test-token".into(),
            phone_number: None,
            email: None,
            is_admin: false,
            can_create_league: false,
            league_id: user.league_id,
            language: "de".into(),
        },
        "classic",
    );

    // One exact finished tip → at least one earned achievement chip.
    h.predictions.seed_finished(FakeFinishedRow {
        user_id: user.id,
        league_id: user.league_id,
        match_id: 1,
        stage: Stage::Group,
        kickoff: chrono::Utc::now() - chrono::Duration::hours(3),
        score_home: 0,
        score_away: 0,
        predicted_home: 0,
        predicted_away: 0,
    });

    let name_lower = user.name.to_lowercase();
    let response = jersey_post(
        State(h.state),
        user,
        Query(JerseyPostQuery {
            preset: "brasilien".into(),
        }),
    )
    .await
    .expect("jersey_post should succeed");

    let html = response.0;

    assert!(
        html.contains(&format!("leaderboard-entry-{name_lower}")),
        "OOB fragment must target the user's leaderboard row"
    );
    assert!(
        html.contains("cursor:help"),
        "OOB-refreshed row must still render the badge chips"
    );
}
