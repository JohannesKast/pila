//! Tests for `POST /profile/language` (language preference handler).

use std::sync::Arc;

use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use pila::auth::AuthenticatedUser;
use pila::handlers::jersey::{set_language_post, SetLanguageForm};
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
    users: Arc<MemoryUserRepo>,
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
    let token = "test-token".to_string();
    users
        .create(NewUser {
            id: user_id,
            name: "Tester",
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

    Harness {
        state,
        users,
        user_id,
        token,
    }
}

fn fake_user(id: Uuid) -> AuthenticatedUser {
    AuthenticatedUser {
        id,
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

async fn call_set_language(h: &Harness, lang: &str) -> axum::response::Response {
    set_language_post(
        State(h.state.clone()),
        fake_user(h.user_id),
        Form(SetLanguageForm {
            language: lang.into(),
        }),
    )
    .await
    .into_response()
}

// ─── Accepted locales ─────────────────────────────────────────────────────────

#[tokio::test]
async fn set_language_de_returns_ok_with_hx_location() {
    let h = build_harness().await;
    let res = call_set_language(&h, "de").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("HX-Location").unwrap(), "/");
}

#[tokio::test]
async fn set_language_en_returns_ok_with_hx_location() {
    let h = build_harness().await;
    let res = call_set_language(&h, "en").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("HX-Location").unwrap(), "/");
}

#[tokio::test]
async fn set_language_es_returns_ok_with_hx_location() {
    let h = build_harness().await;
    let res = call_set_language(&h, "es").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("HX-Location").unwrap(), "/");
}

#[tokio::test]
async fn set_language_fr_returns_ok_with_hx_location() {
    let h = build_harness().await;
    let res = call_set_language(&h, "fr").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("HX-Location").unwrap(), "/");
}

// ─── Rejected locales ─────────────────────────────────────────────────────────

#[tokio::test]
async fn set_language_unknown_locale_returns_bad_request() {
    let h = build_harness().await;
    let res = call_set_language(&h, "xx").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn set_language_empty_locale_returns_bad_request() {
    let h = build_harness().await;
    let res = call_set_language(&h, "").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// ─── Persistence ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn set_language_persists_new_locale_to_repo() {
    let h = build_harness().await;
    let res = call_set_language(&h, "en").await;
    assert_eq!(res.status(), StatusCode::OK);

    let user = h
        .users
        .find_by_token(&h.token)
        .await
        .unwrap()
        .expect("user must exist");
    assert_eq!(user.language, "en");
}
