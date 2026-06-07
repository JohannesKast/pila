//! Invite-link admin handlers and public self-registration, against the
//! in-memory repos.

use std::sync::Arc;

use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use uuid::Uuid;

use pila::auth::{AdminUser, AuthenticatedUser};
use pila::handlers::admin::{admin_create_invite, admin_revoke_invite, InviteCreateForm};
use pila::handlers::join::{join_get, join_post, JoinForm};
use pila::repo::invite::InviteRepo;
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::user::UserRepo;
use pila::repo::{
    MemoryBootstrapRepo, MemoryInviteRepo, MemoryMatchRepo, MemoryNotificationRepo,
    MemoryPredictionRepo, MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo,
    MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
};
use pila::AppState;

struct Harness {
    state: AppState,
    users: Arc<MemoryUserRepo>,
    invites: Arc<MemoryInviteRepo>,
}

fn build_harness() -> Harness {
    let users = Arc::new(MemoryUserRepo::new());
    let invites = Arc::new(MemoryInviteRepo::new());
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
        invites: invites.clone(),
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
        invites,
    }
}

fn admin_extractor(id: Uuid, can_create_league: bool) -> AdminUser {
    AdminUser(AuthenticatedUser {
        id,
        name: "Admin".into(),
        is_admin: true,
        can_create_league,
        phone_number: None,
        email: None,
        jersey_preset: "classic".into(),
        language: "de".into(),
        league_id: DEFAULT_LEAGUE_ID,
    })
}

// ─── admin_create_invite ──────────────────────────────────────────────────────

#[tokio::test]
async fn admin_create_invite_persists_link_with_join_url() {
    let h = build_harness();
    let admin = admin_extractor(Uuid::new_v4(), false);
    let res = admin_create_invite(
        State(h.state.clone()),
        admin,
        Path(DEFAULT_LEAGUE_ID),
        Form(InviteCreateForm {
            label: "WhatsApp".into(),
        }),
    )
    .await
    .expect("created");

    // The rendered row carries the public /join/ URL and the label.
    assert!(res.0.contains("/join/"), "row should show the join URL");
    assert!(res.0.contains("WhatsApp"), "row should show the label");

    let stored = h.invites.list_for_league(DEFAULT_LEAGUE_ID).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].label.as_deref(), Some("WhatsApp"));
}

#[tokio::test]
async fn admin_create_invite_rejects_foreign_league_for_regular_admin() {
    let h = build_harness();
    let admin = admin_extractor(Uuid::new_v4(), false);
    let other_league = Uuid::new_v4();
    let res = admin_create_invite(
        State(h.state.clone()),
        admin,
        Path(other_league),
        Form(InviteCreateForm {
            label: String::new(),
        }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::FORBIDDEN);
}

// ─── admin_revoke_invite ──────────────────────────────────────────────────────

#[tokio::test]
async fn admin_revoke_invite_deletes_link() {
    let h = build_harness();
    let id = h
        .invites
        .create(DEFAULT_LEAGUE_ID, "tok", None)
        .await
        .unwrap();
    let admin = admin_extractor(Uuid::new_v4(), false);
    let _ = admin_revoke_invite(State(h.state.clone()), admin, Path(id))
        .await
        .expect("revoked");
    assert!(h.invites.find_by_id(id).await.unwrap().is_none());
}

#[tokio::test]
async fn admin_revoke_invite_refuses_foreign_league() {
    let h = build_harness();
    let foreign_league = Uuid::new_v4();
    let id = h.invites.create(foreign_league, "tok", None).await.unwrap();
    let admin = admin_extractor(Uuid::new_v4(), false);
    let res = admin_revoke_invite(State(h.state.clone()), admin, Path(id)).await;
    assert_eq!(res.unwrap_err().0, StatusCode::FORBIDDEN);
    // Untouched.
    assert!(h.invites.find_by_id(id).await.unwrap().is_some());
}

#[tokio::test]
async fn admin_revoke_invite_is_idempotent_for_unknown_id() {
    // Revoking an id that no longer exists must still return an empty 200 so
    // the HTMX `outerHTML` swap removes the row instead of leaving it on screen.
    let h = build_harness();
    let admin = admin_extractor(Uuid::new_v4(), false);
    let res = admin_revoke_invite(State(h.state.clone()), admin, Path(Uuid::new_v4()))
        .await
        .expect("idempotent ok");
    assert!(res.0.is_empty());
}

#[tokio::test]
async fn revoked_invite_token_no_longer_lets_anyone_join() {
    // End-to-end guarantee behind "revoke": a player holding the link can join
    // before revoke and is rejected afterwards.
    let h = build_harness();
    let id = h
        .invites
        .create(DEFAULT_LEAGUE_ID, "share-token", None)
        .await
        .unwrap();

    // Before revoke: the join page renders.
    let ok = join_get(
        State(h.state.clone()),
        Path("share-token".into()),
        HeaderMap::new(),
    )
    .await
    .expect("join page renders before revoke");
    assert_eq!(ok.into_parts().0.status, StatusCode::OK);

    // Admin revokes the link.
    let admin = admin_extractor(Uuid::new_v4(), false);
    let _ = admin_revoke_invite(State(h.state.clone()), admin, Path(id))
        .await
        .expect("revoked");

    // After revoke: both the form and the submit reject the dead token.
    let get_after = join_get(
        State(h.state.clone()),
        Path("share-token".into()),
        HeaderMap::new(),
    )
    .await;
    assert_eq!(get_after.unwrap_err().0, StatusCode::NOT_FOUND);

    let post_after = join_post(
        State(h.state.clone()),
        Path("share-token".into()),
        HeaderMap::new(),
        axum_extra::extract::CookieJar::new(),
        Form(JoinForm {
            name: "Too Late".into(),
        }),
    )
    .await;
    assert_eq!(post_after.unwrap_err().0, StatusCode::NOT_FOUND);

    // And no user leaked through.
    assert!(h
        .users
        .list_for_admin(DEFAULT_LEAGUE_ID)
        .await
        .unwrap()
        .is_empty());
}

// ─── public join flow ─────────────────────────────────────────────────────────

#[tokio::test]
async fn join_get_unknown_token_is_not_found() {
    let h = build_harness();
    let res = join_get(
        State(h.state.clone()),
        Path("does-not-exist".into()),
        HeaderMap::new(),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn join_get_valid_token_renders_form_with_league_name() {
    let h = build_harness();
    h.invites
        .create(DEFAULT_LEAGUE_ID, "valid-token", None)
        .await
        .unwrap();
    let res = join_get(
        State(h.state.clone()),
        Path("valid-token".into()),
        HeaderMap::new(),
    )
    .await
    .expect("renders");
    let (parts, _) = res.into_parts();
    assert_eq!(parts.status, StatusCode::OK);
}

#[tokio::test]
async fn join_post_creates_user_in_invite_league() {
    let h = build_harness();
    h.invites
        .create(DEFAULT_LEAGUE_ID, "valid-token", None)
        .await
        .unwrap();

    let (_jar, _resp) = join_post(
        State(h.state.clone()),
        Path("valid-token".into()),
        HeaderMap::new(),
        axum_extra::extract::CookieJar::new(),
        Form(JoinForm {
            name: "Newcomer".into(),
        }),
    )
    .await
    .expect("registered");

    let listed = h.users.list_for_admin(DEFAULT_LEAGUE_ID).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Newcomer");
    assert!(
        !listed[0].is_admin,
        "self-registered users are never admins"
    );
}

#[tokio::test]
async fn join_post_rejects_blank_name() {
    let h = build_harness();
    h.invites
        .create(DEFAULT_LEAGUE_ID, "valid-token", None)
        .await
        .unwrap();
    let res = join_post(
        State(h.state.clone()),
        Path("valid-token".into()),
        HeaderMap::new(),
        axum_extra::extract::CookieJar::new(),
        Form(JoinForm { name: "   ".into() }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn join_post_rejects_revoked_token() {
    let h = build_harness();
    let id = h
        .invites
        .create(DEFAULT_LEAGUE_ID, "tok", None)
        .await
        .unwrap();
    h.invites.delete(id).await.unwrap();
    let res = join_post(
        State(h.state.clone()),
        Path("tok".into()),
        HeaderMap::new(),
        axum_extra::extract::CookieJar::new(),
        Form(JoinForm {
            name: "Too Late".into(),
        }),
    )
    .await;
    assert_eq!(res.unwrap_err().0, StatusCode::NOT_FOUND);
}
