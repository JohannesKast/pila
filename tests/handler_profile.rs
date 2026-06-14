//! Tests for the self-service profile editor (`GET /profile/name-editor`,
//! `POST /profile/name`).
//!
//! Covers the behaviours that matter for the private-real-name feature: opening
//! the editor pre-fills both current names; a successful rename persists both
//! names and navigates home; a blank real name falls back to the tip name; an
//! empty or over-long tip name re-renders the sheet without mutating the user;
//! and a tip name already taken by another player in the league is rejected.

use std::sync::Arc;

use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use pila::auth::AuthenticatedUser;
use pila::handlers::profile::{profile_editor_get, profile_name_post, ProfileNameForm};
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

fn caller(id: Uuid) -> AuthenticatedUser {
    caller_named(id, "Tester", "Tester")
}

fn caller_named(id: Uuid, name: &str, real_name: &str) -> AuthenticatedUser {
    AuthenticatedUser {
        id,
        name: name.into(),
        real_name: real_name.into(),
        is_admin: false,
        can_create_league: false,
        phone_number: None,
        email: None,
        jersey_preset: "classic".into(),
        language: "de".into(),
        league_id: DEFAULT_LEAGUE_ID,
    }
}

async fn post(h: &Harness, name: &str, real_name: &str) -> axum::response::Response {
    profile_name_post(
        State(h.state.clone()),
        caller(h.user_id),
        Form(ProfileNameForm {
            name: name.into(),
            real_name: real_name.into(),
        }),
    )
    .await
    .into_response()
}

#[tokio::test]
async fn saving_both_names_persists_and_navigates_home() {
    let h = build_harness().await;
    let res = post(&h, "TippKing", "Maximilian").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("HX-Location").unwrap(), "/");

    let user = h.users.find_by_token(&h.token).await.unwrap().unwrap();
    assert_eq!(user.name, "TippKing");
    assert_eq!(user.real_name, "Maximilian");
}

#[tokio::test]
async fn blank_real_name_defaults_to_tip_name() {
    let h = build_harness().await;
    let res = post(&h, "SoloTipper", "   ").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("HX-Location").unwrap(), "/");

    let user = h.users.find_by_token(&h.token).await.unwrap().unwrap();
    assert_eq!(user.name, "SoloTipper");
    assert_eq!(user.real_name, "SoloTipper");
}

#[tokio::test]
async fn empty_tip_name_re_renders_sheet_without_navigation() {
    let h = build_harness().await;
    let res = post(&h, "   ", "Maximilian").await;
    // The sheet is re-rendered (200) with an inline error, but no navigation
    // happens and nothing is persisted.
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("HX-Location").is_none());

    let user = h.users.find_by_token(&h.token).await.unwrap().unwrap();
    assert_eq!(user.name, "Tester");
    assert_eq!(user.real_name, "Tester");
}

#[tokio::test]
async fn opening_editor_prefills_both_current_names() {
    let h = build_harness().await;
    let res = profile_editor_get(
        State(h.state.clone()),
        caller_named(h.user_id, "PublicTip", "SecretReal"),
    )
    .await;
    // The sheet pre-fills both inputs with the user's current values so an edit
    // starts from what they already have.
    assert!(res.0.contains("PublicTip"), "tip name not prefilled");
    assert!(res.0.contains("SecretReal"), "real name not prefilled");
}

#[tokio::test]
async fn overly_long_tip_name_is_rejected_without_persisting() {
    let h = build_harness().await;
    let too_long = "x".repeat(256);
    let res = post(&h, &too_long, "Maximilian").await;
    // Over the 255-char limit: the sheet re-renders (200) with no navigation
    // and the caller's stored names stay untouched.
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("HX-Location").is_none());

    let user = h.users.find_by_token(&h.token).await.unwrap().unwrap();
    assert_eq!(user.name, "Tester");
    assert_eq!(user.real_name, "Tester");
}

#[tokio::test]
async fn tip_name_taken_by_other_player_is_rejected() {
    let h = build_harness().await;
    h.users
        .create(NewUser {
            id: Uuid::new_v4(),
            name: "Alice",
            real_name: "Alice",
            token: "other-token",
            is_admin: false,
            phone_number: None,
            email: None,
            league_id: DEFAULT_LEAGUE_ID,
            language: "de",
        })
        .await
        .unwrap();

    // Case-insensitive collision: the editor re-renders the sheet and leaves
    // the caller's own names untouched.
    let res = post(&h, "alice", "Maximilian").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("HX-Location").is_none());

    let user = h.users.find_by_token(&h.token).await.unwrap().unwrap();
    assert_eq!(user.name, "Tester");
    assert_eq!(user.real_name, "Tester");
}
