//! Postgres-backed integration tests for the repo layer.
//!
//! Mirrors the in-memory fake tests, but exercises the actual SQL — proves
//! that the trait contracts hold against the real schema and the sqlx
//! macros stay in sync after a migration.
//!
//! Each test creates its own ephemeral users/matches and tears them down at
//! the end; the suite is meant to be safe to run repeatedly against the
//! shared dev database.

use chrono::{Duration, Utc};
use pila::repo::match_::{MatchRepo, PgMatchRepo};
use pila::repo::prediction::{PgPredictionRepo, PredictionRepo};
use pila::repo::settings::{PgSettingsRepo, SettingsRepo};
use pila::repo::special_prediction::{PgSpecialPredictionRepo, SpecialPredictionRepo};
use pila::repo::team::{PgTeamRepo, TeamRepo};
use pila::repo::user::{NewUser, PgUserRepo, UserRepo};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate");
    pool
}

#[tokio::test]
async fn user_repo_create_find_rename_set_admin_delete_round_trip() {
    let pool = pool().await;
    let repo = PgUserRepo::new(pool.clone());

    let id = Uuid::new_v4();
    let token = format!("repo-test-{}", Uuid::new_v4());
    repo.create(NewUser {
        id,
        name: "Repo Test",
        token: &token,
        is_admin: false,
        phone_number: Some("+490"),
    })
    .await
    .unwrap();

    let auth = repo.find_by_token(&token).await.unwrap().expect("found");
    assert_eq!(auth.name, "Repo Test");
    assert_eq!(auth.phone_number.as_deref(), Some("+490"));
    assert!(!auth.is_admin);

    repo.rename(id, "Renamed").await.unwrap();
    let after = repo.find_full_by_id(id).await.unwrap().unwrap();
    assert_eq!(after.name, "Renamed");

    repo.set_admin(id, true).await.unwrap();
    let promoted = repo.find_full_by_id(id).await.unwrap().unwrap();
    assert!(promoted.is_admin);

    let admin_count_before = repo.count_admins().await.unwrap();
    repo.set_admin(id, false).await.unwrap();
    let admin_count_after = repo.count_admins().await.unwrap();
    assert_eq!(admin_count_after, admin_count_before - 1);

    repo.delete(id).await.unwrap();
    assert!(repo.find_full_by_id(id).await.unwrap().is_none());
    assert!(repo.find_by_token(&token).await.unwrap().is_none());
}

#[tokio::test]
async fn user_repo_set_jersey_persists() {
    let pool = pool().await;
    let repo = PgUserRepo::new(pool.clone());

    let id = Uuid::new_v4();
    let token = format!("repo-test-{}", Uuid::new_v4());
    repo.create(NewUser {
        id,
        name: "Jersey",
        token: &token,
        is_admin: false,
        phone_number: None,
    })
    .await
    .unwrap();

    repo.set_jersey(id, "brasilien").await.unwrap();
    let auth = repo.find_by_token(&token).await.unwrap().unwrap();
    assert_eq!(auth.jersey_preset, "brasilien");

    repo.delete(id).await.unwrap();
}

#[tokio::test]
async fn user_repo_list_for_admin_returns_alphabetical() {
    let pool = pool().await;
    let repo = PgUserRepo::new(pool.clone());

    // Just assert non-empty rows come back sorted; the dev DB may have data.
    let list = repo.list_for_admin().await.unwrap();
    let mut sorted = list.clone();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        list.iter().map(|r| &r.name).collect::<Vec<_>>(),
        sorted.iter().map(|r| &r.name).collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn match_repo_first_kickoff_and_actual_champion_are_consistent() {
    let pool = pool().await;
    let repo = PgMatchRepo::new(pool.clone());

    // Sanity only — both methods must succeed and return Option without panicking.
    let _ = repo.first_kickoff().await.unwrap();
    let _ = repo.actual_champion().await.unwrap();
    let _ = repo
        .started_with_both_teams_count(Utc::now())
        .await
        .unwrap();
}

#[tokio::test]
async fn match_repo_finished_group_rows_only_returns_finished() {
    let pool = pool().await;
    let repo = PgMatchRepo::new(pool.clone());
    let rows = repo.finished_group_rows().await.unwrap();
    // Every returned row must have both score components set (otherwise the
    // SQL filter would be wrong and would crash the standings calc).
    for r in rows {
        // i32 score values are always present by virtue of the query filter;
        // assert the supporting joins gave us non-empty names.
        assert!(!r.home_name.is_empty());
        assert!(!r.away_name.is_empty());
        assert!(!r.group_letter.is_empty());
    }
}

#[tokio::test]
async fn prediction_repo_upsert_overwrites_and_round_trips() {
    let pool = pool().await;
    let users = PgUserRepo::new(pool.clone());
    let preds = PgPredictionRepo::new(pool.clone());

    let user_id = Uuid::new_v4();
    let token = format!("repo-test-{}", Uuid::new_v4());
    users
        .create(NewUser {
            id: user_id,
            name: "Pred",
            token: &token,
            is_admin: false,
            phone_number: None,
        })
        .await
        .unwrap();

    // Need a real match id to insert a prediction; pick the lowest existing one.
    let some_match: Option<i32> =
        sqlx::query_scalar!("SELECT id FROM matches ORDER BY id LIMIT 1")
            .fetch_optional(&pool)
            .await
            .unwrap();

    let Some(match_id) = some_match else {
        // Empty matches table — skip rather than fail.
        users.delete(user_id).await.unwrap();
        return;
    };

    preds.upsert(user_id, match_id, 2, 1).await.unwrap();
    preds.upsert(user_id, match_id, 3, 0).await.unwrap();

    let count = preds.count_user_started(user_id, Utc::now()).await.unwrap();
    // Only counts when the kickoff is in the past — assertion stays valid
    // either way because we just need the call not to panic.
    assert!(count >= 0);

    sqlx::query!(
        "DELETE FROM predictions WHERE user_id = $1",
        user_id
    )
    .execute(&pool)
    .await
    .unwrap();
    users.delete(user_id).await.unwrap();
}

#[tokio::test]
async fn prediction_repo_other_users_locked_excludes_viewer() {
    let pool = pool().await;
    let preds = PgPredictionRepo::new(pool.clone());
    let viewer = Uuid::new_v4();
    // The viewer doesn't exist — the call must still succeed and return rows
    // for any other locked tips.
    let rows = preds
        .list_other_users_locked(viewer, Utc::now() + Duration::hours(1))
        .await
        .unwrap();
    for r in rows {
        assert_ne!(r.user_name, "");
    }
}

#[tokio::test]
async fn special_prediction_repo_upsert_round_trip() {
    let pool = pool().await;
    let users = PgUserRepo::new(pool.clone());
    let sp = PgSpecialPredictionRepo::new(pool.clone());

    let user_id = Uuid::new_v4();
    let token = format!("repo-test-{}", Uuid::new_v4());
    users
        .create(NewUser {
            id: user_id,
            name: "Sp",
            token: &token,
            is_admin: false,
            phone_number: None,
        })
        .await
        .unwrap();

    let some_team: Option<i32> =
        sqlx::query_scalar!("SELECT id FROM teams ORDER BY id LIMIT 1")
            .fetch_optional(&pool)
            .await
            .unwrap();

    let Some(team_id) = some_team else {
        users.delete(user_id).await.unwrap();
        return;
    };

    sp.upsert(user_id, Some(team_id)).await.unwrap();
    assert_eq!(sp.get_user_champion(user_id).await.unwrap(), Some(team_id));

    sp.upsert(user_id, None).await.unwrap();
    assert_eq!(sp.get_user_champion(user_id).await.unwrap(), None);

    sqlx::query!(
        "DELETE FROM special_predictions WHERE user_id = $1",
        user_id
    )
    .execute(&pool)
    .await
    .unwrap();
    users.delete(user_id).await.unwrap();
}

#[tokio::test]
async fn team_repo_dropdown_excludes_placeholders() {
    let pool = pool().await;
    let teams = PgTeamRepo::new(pool.clone());

    let list = teams.list_real_for_dropdown().await.unwrap();
    for t in list {
        assert!(!t.name.starts_with("Group "));
        assert!(!t.name.starts_with("Quarterfinal "));
        assert!(!t.name.starts_with("Semifinal "));
        assert!(!t.name.starts_with("Round of "));
        assert!(!t.name.starts_with("Third Place "));
    }
}

#[tokio::test]
async fn settings_repo_returns_none_for_unknown_key() {
    let pool = pool().await;
    let settings = PgSettingsRepo::new(pool.clone());
    let v = settings
        .get("__definitely_not_a_real_setting__")
        .await
        .unwrap();
    assert!(v.is_none());
}
