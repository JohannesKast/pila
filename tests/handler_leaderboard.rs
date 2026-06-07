//! Tests for `GET /leaderboard`.
//!
//! Verifies that the handler returns HTML, includes the caller's league-mates,
//! and excludes users from other leagues.

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::State;
use axum::response::IntoResponse;
use uuid::Uuid;

use pila::auth::AuthenticatedUser;
use pila::handlers::leaderboard::leaderboard;
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::user::NewUser;
use pila::repo::UserRepo;
use pila::repo::{
    MemoryBootstrapRepo, MemoryInviteRepo, MemoryMatchRepo, MemoryNotificationRepo,
    MemoryPredictionRepo, MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo,
    MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
};
use pila::AppState;

const OTHER_LEAGUE_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000002);

struct Harness {
    state: AppState,
    users: Arc<MemoryUserRepo>,
}

fn build_harness() -> Harness {
    let users = Arc::new(MemoryUserRepo::new());
    let leagues = Arc::new(MemoryLeagueRepo::new());
    leagues.seed(League {
        id: DEFAULT_LEAGUE_ID,
        name: "League A".into(),
        notifications_bootstrapped: true,
    });
    leagues.seed(League {
        id: OTHER_LEAGUE_ID,
        name: "League B".into(),
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

async fn seed_user(users: &MemoryUserRepo, name: &str, league_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    users
        .create(NewUser {
            id,
            name,
            token: &Uuid::new_v4().to_string(),
            is_admin: false,
            phone_number: None,
            email: None,
            league_id,
            language: "de",
        })
        .await
        .unwrap();
    id
}

fn caller(id: Uuid, league_id: Uuid) -> AuthenticatedUser {
    AuthenticatedUser {
        id,
        name: "Caller".into(),
        is_admin: false,
        can_create_league: false,
        phone_number: None,
        email: None,
        jersey_preset: "classic".into(),
        language: "de".into(),
        league_id,
    }
}

async fn body_string(state: AppState, user: AuthenticatedUser) -> String {
    let html = leaderboard(State(state), user).await;
    let bytes = to_bytes(html.into_response().into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ─── Basic rendering ──────────────────────────────────────────────────────────

#[tokio::test]
async fn leaderboard_returns_html_with_no_users() {
    let h = build_harness();
    let alice_id = seed_user(&h.users, "Alice", DEFAULT_LEAGUE_ID).await;
    let body = body_string(h.state, caller(alice_id, DEFAULT_LEAGUE_ID)).await;
    assert!(!body.is_empty(), "leaderboard should render non-empty HTML");
}

#[tokio::test]
async fn leaderboard_includes_users_from_same_league() {
    let h = build_harness();
    let alice_id = seed_user(&h.users, "Alice", DEFAULT_LEAGUE_ID).await;
    seed_user(&h.users, "Bob", DEFAULT_LEAGUE_ID).await;
    let body = body_string(h.state, caller(alice_id, DEFAULT_LEAGUE_ID)).await;
    assert!(body.contains("Alice"), "leaderboard should contain Alice");
    assert!(body.contains("Bob"), "leaderboard should contain Bob");
}

// ─── League isolation ─────────────────────────────────────────────────────────

#[tokio::test]
async fn leaderboard_excludes_users_from_other_league() {
    let h = build_harness();
    let alice_id = seed_user(&h.users, "Alice", DEFAULT_LEAGUE_ID).await;
    seed_user(&h.users, "Charlie", OTHER_LEAGUE_ID).await;

    let body = body_string(h.state, caller(alice_id, DEFAULT_LEAGUE_ID)).await;
    assert!(body.contains("Alice"), "Alice (same league) should appear");
    assert!(
        !body.contains("Charlie"),
        "Charlie (other league) must not appear"
    );
}

#[tokio::test]
async fn leaderboard_scoped_to_callers_league() {
    let h = build_harness();
    seed_user(&h.users, "Alice", DEFAULT_LEAGUE_ID).await;
    let charlie_id = seed_user(&h.users, "Charlie", OTHER_LEAGUE_ID).await;

    let body = body_string(h.state, caller(charlie_id, OTHER_LEAGUE_ID)).await;
    assert!(
        body.contains("Charlie"),
        "Charlie's leaderboard should contain Charlie"
    );
    assert!(
        !body.contains("Alice"),
        "Alice (different league) must not appear in Charlie's leaderboard"
    );
}
