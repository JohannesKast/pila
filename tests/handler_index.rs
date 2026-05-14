//! Regression test for the blank-dashboard bug caused by a `<head>` script
//! referencing `document.body` before the `<body>` tag is parsed.  The
//! thrown error blocked *all* inline scripts, so the tab-panels stayed
//! `hidden` and the HTMX CSRF-token listener was never registered.

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::State;
use uuid::Uuid;

use pila::auth::MaybeAuthenticatedUser;
use pila::handlers::index;
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::user::UserFull;
use pila::repo::{
    MemoryMatchRepo, MemoryNotificationRepo, MemoryPredictionRepo, MemorySettingsRepo,
    MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
};
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
        users: users.clone(),
        leagues,
        matches: Arc::new(MemoryMatchRepo::new()),
        predictions: Arc::new(MemoryPredictionRepo::new()),
        special_predictions: Arc::new(MemorySpecialPredictionRepo::new()),
        teams: Arc::new(MemoryTeamRepo::new()),
        settings: Arc::new(MemorySettingsRepo::new()),
        notifications: Arc::new(MemoryNotificationRepo::new()),
    };

    let state = AppState {
        jerseys: pila::jersey::load(),
        news: pila::news::NewsCache::from_env(),
        repos,
        translations: pila::translations::load_all(),
        concurrency_limit: Arc::new(tokio::sync::Semaphore::new(100)),
        db: None,
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
            token: "test-token".into(),
            phone_number: None,
        email: None,
            is_admin: false,
            can_create_league: false,
            league_id: user.league_id,
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
