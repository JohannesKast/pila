//! Tests for authentication flows:
//!   - `GET /play/me/:token` magic-link handler
//!   - `AuthenticatedUser` extractor (valid / missing / unknown token)

use std::sync::Arc;

use axum::extract::{FromRequestParts, Path, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use pila::auth::AuthenticatedUser;
use pila::handlers::auth::login_magic_link;
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::user::NewUser;
use pila::repo::UserRepo;
use pila::repo::{
    MemoryBootstrapRepo, MemoryInviteRepo, MemoryMatchRepo, MemoryNotificationRepo,
    MemoryPredictionRepo, MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo,
    MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
};
use pila::AppState;

struct Harness {
    state: AppState,
    user_id: Uuid,
    token: String,
}

async fn build_harness() -> Harness {
    let users = Arc::new(MemoryUserRepo::new());
    let leagues = Arc::new(MemoryLeagueRepo::new());
    leagues.seed(League {
        id: DEFAULT_LEAGUE_ID,
        name: "Default".into(),
        notifications_bootstrapped: true,
    });

    let user_id = Uuid::new_v4();
    let token = Uuid::new_v4().to_string();
    users
        .create(NewUser {
            id: user_id,
            name: "Tester",
            real_name: "Tester",
            token: &token,
            is_admin: false,
            phone_number: None,
            email: None,
            league_id: DEFAULT_LEAGUE_ID,
            language: "de",
        })
        .await
        .unwrap();

    let repos = Repos {
        bootstrap: Arc::new(MemoryBootstrapRepo::new()),
        users,
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

    Harness {
        state,
        user_id,
        token,
    }
}

// ─── login_magic_link ────────────────────────────────────────────────────────

#[tokio::test]
async fn magic_link_valid_token_sets_cookie_and_redirects() {
    let h = build_harness().await;
    let (jar, redirect) = login_magic_link(
        State(h.state),
        Path(h.token),
        HeaderMap::new(),
        CookieJar::default(),
    )
    .await
    .expect("valid token should succeed");

    assert!(
        jar.get("pila_token").is_some(),
        "pila_token cookie must be set"
    );
    // Redirect target is "/"
    let location = redirect.into_response().headers()["location"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(location, "/");
}

#[tokio::test]
async fn magic_link_unknown_token_returns_401() {
    let h = build_harness().await;
    let (status, _) = login_magic_link(
        State(h.state),
        Path("no-such-token".to_string()),
        HeaderMap::new(),
        CookieJar::default(),
    )
    .await
    .expect_err("unknown token should fail");

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ─── AuthenticatedUser extractor ─────────────────────────────────────────────

async fn extract_user(
    state: &AppState,
    cookie_header: Option<&str>,
) -> Result<AuthenticatedUser, (StatusCode, String)> {
    let mut builder = Request::builder();
    if let Some(val) = cookie_header {
        builder = builder.header("cookie", val);
    }
    let req = builder.body(()).unwrap();
    let (mut parts, _) = req.into_parts();
    AuthenticatedUser::from_request_parts(&mut parts, state).await
}

#[tokio::test]
async fn auth_extractor_valid_cookie_returns_user() {
    let h = build_harness().await;
    let cookie = format!("pila_token={}", h.token);
    let user = extract_user(&h.state, Some(&cookie))
        .await
        .expect("valid token should authenticate");

    assert_eq!(user.id, h.user_id);
    assert_eq!(user.name, "Tester");
    assert_eq!(user.league_id, DEFAULT_LEAGUE_ID);
}

#[tokio::test]
async fn auth_extractor_missing_cookie_returns_401() {
    let h = build_harness().await;
    let Err((status, _)) = extract_user(&h.state, None).await else {
        panic!("missing cookie should fail");
    };

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_extractor_unknown_token_returns_401() {
    let h = build_harness().await;
    let Err((status, _)) = extract_user(&h.state, Some("pila_token=not-a-real-token")).await else {
        panic!("unknown token should fail");
    };

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_extractor_returns_correct_user_fields() {
    let h = build_harness().await;
    let cookie = format!("pila_token={}", h.token);
    let user = extract_user(&h.state, Some(&cookie)).await.unwrap();

    assert!(!user.is_admin);
    assert!(!user.can_create_league);
    assert_eq!(user.language, "de");
    assert_eq!(user.jersey_preset, "classic");
}
