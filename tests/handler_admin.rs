//! Admin handler tests against the in-memory repos.

use std::sync::Arc;

use axum::extract::{Form, Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use pila::auth::{AdminUser, AuthenticatedUser};
use pila::handlers::admin::{
    admin_create_user, admin_delete_user, admin_rename_user, admin_toggle_admin, AdminCreateForm,
    AdminRenameForm,
};
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::user::{NewUser, UserFull};
use pila::repo::{
    MemoryMatchRepo, MemoryNotificationRepo, MemoryPredictionRepo, MemorySettingsRepo,
    MemorySpecialPredictionRepo, MemoryTeamRepo, MemoryUserRepo, Repos, UserRepo,
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
        translations: std::collections::HashMap::new(),
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

fn admin_extractor(id: Uuid) -> AdminUser {
    AdminUser(AuthenticatedUser {
        id,
        name: "Admin".into(),
        is_admin: true,
        can_create_league: false,
        phone_number: None,
        email: None,
        jersey_preset: "classic".into(),
        language: "de".into(),
        league_id: DEFAULT_LEAGUE_ID,
    })
}

fn user_full(id: Uuid, name: &str, token: &str, is_admin: bool) -> UserFull {
    UserFull {
        id,
        name: name.into(),
        token: token.into(),
        phone_number: None,
        email: None,
        is_admin,
        can_create_league: false,
        league_id: DEFAULT_LEAGUE_ID,
        language: "de".into(),
    }
}

// ─── admin_create_user ────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_create_user_rejects_blank_name() {
    let h = build_harness();
    let admin = admin_extractor(Uuid::new_v4());
    let res = admin_create_user(
        State(h.state.clone()),
        admin,
        Path(DEFAULT_LEAGUE_ID),
        Form(AdminCreateForm {
            name: "   ".into(),
            phone_number: String::new(),
            email: "".to_string(),
        }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_create_user_persists_new_user_and_returns_row_html() {
    let h = build_harness();
    let admin = admin_extractor(Uuid::new_v4());
    let res = admin_create_user(
        State(h.state.clone()),
        admin,
        Path(DEFAULT_LEAGUE_ID),
        Form(AdminCreateForm {
            name: "Bob".into(),
            phone_number: String::new(),
            email: "".to_string(),
        }),
    )
    .await
    .expect("created");
    let body = res.0;
    assert!(body.contains("Bob"), "html should mention the new user's name");

    let listed = h.users.list_for_admin(DEFAULT_LEAGUE_ID).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Bob");
    assert!(!listed[0].is_admin, "non-self-promotion path");
}

// ─── admin_delete_user ────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_delete_user_refuses_self() {
    let h = build_harness();
    let admin_id = Uuid::new_v4();
    let admin = admin_extractor(admin_id);
    let res = admin_delete_user(State(h.state.clone()), admin, Path(admin_id)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_delete_user_removes_target() {
    let h = build_harness();
    let target_id = Uuid::new_v4();
    h.users
        .create(NewUser {
            id: target_id,
            name: "Target",
            token: "tk",
            is_admin: false,
            phone_number: None,
            league_id: DEFAULT_LEAGUE_ID,
            language: "de",
        email: None,
        })
        .await
        .unwrap();
    let admin = admin_extractor(Uuid::new_v4());
    let _ = admin_delete_user(State(h.state.clone()), admin, Path(target_id))
        .await
        .unwrap();
    assert!(h.users.find_full_by_id(target_id).await.unwrap().is_none());
}

// ─── admin_toggle_admin ───────────────────────────────────────────────────────

#[tokio::test]
async fn admin_toggle_admin_promotes_target() {
    let h = build_harness();
    let target_id = Uuid::new_v4();
    h.users.seed(user_full(target_id, "T", "tk", false), "classic");
    let admin = admin_extractor(Uuid::new_v4());
    let _ = admin_toggle_admin(State(h.state.clone()), admin, Path(target_id))
        .await
        .unwrap();
    let after = h.users.find_full_by_id(target_id).await.unwrap().unwrap();
    assert!(after.is_admin);
}

#[tokio::test]
async fn admin_toggle_admin_refuses_to_demote_last_admin() {
    let h = build_harness();
    let admin_id = Uuid::new_v4();
    h.users.seed(user_full(admin_id, "Sole", "tk", true), "classic");
    let admin = admin_extractor(admin_id);
    let res = admin_toggle_admin(State(h.state.clone()), admin, Path(admin_id)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_toggle_admin_refuses_self_demotion_with_other_admins_present() {
    let h = build_harness();
    let admin_id = Uuid::new_v4();
    let other_admin = Uuid::new_v4();
    h.users.seed(user_full(admin_id, "Me", "t1", true), "classic");
    h.users.seed(user_full(other_admin, "Other", "t2", true), "classic");
    let admin = admin_extractor(admin_id);
    let res = admin_toggle_admin(State(h.state.clone()), admin, Path(admin_id)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

// ─── admin_rename_user ────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_rename_user_rejects_blank_name() {
    let h = build_harness();
    let target_id = Uuid::new_v4();
    let admin = admin_extractor(Uuid::new_v4());
    let res = admin_rename_user(
        State(h.state.clone()),
        admin,
        Path(target_id),
        Form(AdminRenameForm {
            name: "  ".into(),
        }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_rename_user_updates_name_and_returns_row() {
    let h = build_harness();
    let target_id = Uuid::new_v4();
    h.users.seed(user_full(target_id, "Old", "tk", false), "classic");
    let admin = admin_extractor(Uuid::new_v4());
    let res = admin_rename_user(
        State(h.state.clone()),
        admin,
        Path(target_id),
        Form(AdminRenameForm {
            name: "New".into(),
        }),
    )
    .await
    .unwrap();
    assert!(res.0.contains("New"));
    let after = h.users.find_full_by_id(target_id).await.unwrap().unwrap();
    assert_eq!(after.name, "New");
}
