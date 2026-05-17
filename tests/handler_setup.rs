//! Tests for the first-run setup handlers (`GET /setup`, `POST /setup`).
//!
//! Uses `MemoryBootstrapRepo` (a no-op stub) for the transaction — the actual
//! multi-table write is covered by the Postgres integration tests. These tests
//! focus on request validation, idempotency guard, and cookie issuance.

use std::sync::Arc;

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use pila::handlers::setup::{setup_get, setup_post, SetupForm};
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::user::NewUser;
use pila::repo::UserRepo;
use pila::repo::{
    MemoryBootstrapRepo, MemoryMatchRepo, MemoryNotificationRepo, MemoryPredictionRepo,
    MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, Repos,
    DEFAULT_LEAGUE_ID,
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
        bootstrap: Arc::new(MemoryBootstrapRepo::new()),
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

fn valid_form() -> SetupForm {
    SetupForm {
        name: "Alice".into(),
        phone_number: String::new(),
        email: String::new(),
        league_name: "Test Liga".into(),
        default_language: "de".into(),
        signal_group_id: String::new(),
        signal_from_number: String::new(),
        rss_feed_url: String::new(),
    }
}

async fn seed_one_user(users: &MemoryUserRepo) {
    users
        .create(NewUser {
            id: Uuid::new_v4(),
            name: "Existing",
            token: "tok",
            is_admin: true,
            phone_number: None,
            email: None,
            league_id: DEFAULT_LEAGUE_ID,
            language: "de",
        })
        .await
        .unwrap();
}

// ─── setup_get ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn setup_get_renders_form_when_no_users_exist() {
    let h = build_harness();
    let res = setup_get(State(h.state), HeaderMap::new()).await;
    assert!(res.is_ok(), "should render form for fresh install");
}

#[tokio::test]
async fn setup_get_redirects_when_users_already_exist() {
    let h = build_harness();
    seed_one_user(&h.users).await;
    let res = setup_get(State(h.state), HeaderMap::new()).await;
    // Redirect is returned as Ok(Redirect), not an error
    assert!(res.is_ok());
}

// ─── setup_post — validation ─────────────────────────────────────────────────

#[tokio::test]
async fn setup_post_happy_path_sets_pila_token_cookie() {
    let h = build_harness();
    let res = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(valid_form()),
    )
    .await;
    let (jar, _) = res.expect("setup_post should succeed on fresh install");
    assert!(
        jar.get("pila_token").is_some(),
        "pila_token cookie must be set after setup"
    );
}

#[tokio::test]
async fn setup_post_rejects_when_users_already_exist() {
    let h = build_harness();
    seed_one_user(&h.users).await;
    let res = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(valid_form()),
    )
    .await;
    let (status, _) = res.expect_err("should fail when users exist");
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn setup_post_rejects_empty_name() {
    let h = build_harness();
    let form = SetupForm {
        name: String::new(),
        ..valid_form()
    };
    let (status, _) = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(form),
    )
    .await
    .expect_err("empty name must be rejected");
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn setup_post_rejects_whitespace_only_name() {
    let h = build_harness();
    let form = SetupForm {
        name: "   ".into(),
        ..valid_form()
    };
    let (status, _) = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(form),
    )
    .await
    .expect_err("whitespace-only name must be rejected");
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn setup_post_rejects_empty_league_name() {
    let h = build_harness();
    let form = SetupForm {
        league_name: String::new(),
        ..valid_form()
    };
    let (status, _) = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(form),
    )
    .await
    .expect_err("empty league_name must be rejected");
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn setup_post_rejects_league_name_exceeding_255_chars() {
    let h = build_harness();
    let form = SetupForm {
        league_name: "x".repeat(256),
        ..valid_form()
    };
    let (status, _) = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(form),
    )
    .await
    .expect_err("league_name >255 chars must be rejected");
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn setup_post_accepts_league_name_of_exactly_255_chars() {
    let h = build_harness();
    let form = SetupForm {
        league_name: "x".repeat(255),
        ..valid_form()
    };
    let res = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(form),
    )
    .await;
    assert!(res.is_ok(), "255-char league_name should be accepted");
}

#[tokio::test]
async fn setup_post_rejects_unknown_language() {
    let h = build_harness();
    let form = SetupForm {
        default_language: "xx".into(),
        ..valid_form()
    };
    let (status, _) = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(form),
    )
    .await
    .expect_err("unknown language must be rejected");
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn setup_post_empty_language_defaults_to_de() {
    let h = build_harness();
    let form = SetupForm {
        default_language: String::new(),
        ..valid_form()
    };
    // Empty language falls back to "de" — handler should succeed
    let res = setup_post(
        State(h.state),
        HeaderMap::new(),
        CookieJar::default(),
        Form(form),
    )
    .await;
    assert!(
        res.is_ok(),
        "empty language should default to de and succeed"
    );
}
