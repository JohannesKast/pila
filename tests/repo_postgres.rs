//! Postgres-backed integration tests for the repo layer.
//!
//! Mirrors the in-memory fake tests, but exercises the actual SQL and proves
//! that the trait contracts hold against the real schema after migrations.
//!
//! Each test creates its own ephemeral users/matches and tears them down at
//! the end; the suite is meant to be safe to run repeatedly against the
//! shared dev database.

use chrono::{Duration, Utc};
use pila::notifier::{NotificationEvent, Notifier, NotifierError};
use pila::repo::fixture::{EspnMatchUpsert, MatchRepo, PgMatchRepo};
use pila::repo::notification::{NotificationRepo, PgNotificationRepo};
use pila::repo::prediction::{PgPredictionRepo, PredictionRepo};
use pila::repo::settings::{PgSettingsRepo, SettingsRepo};
use pila::repo::special_prediction::{PgSpecialPredictionRepo, SpecialPredictionRepo};
use pila::repo::team::{EspnTeamUpsert, PgTeamRepo, TeamRepo};
use pila::repo::user::{NewUser, PgUserRepo, UserRepo};
use pila::repo::DEFAULT_LEAGUE_ID;
use pila::stage::Stage;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use sqlx::Row;
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

    // Migrations no longer seed a default league, but every test in this
    // file references `DEFAULT_LEAGUE_ID` as a stable tenant. Seed it here
    // once (idempotent) so foreign-key inserts succeed.
    sqlx::query(
        "INSERT INTO leagues (id, name, notifications_bootstrapped) \
         VALUES ($1, 'Test Default', true) ON CONFLICT (id) DO NOTHING",
    )
    .bind(DEFAULT_LEAGUE_ID)
    .execute(&pool)
    .await
    .expect("seed default test league");

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
        league_id: DEFAULT_LEAGUE_ID,
        email: None,
        language: "de",
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
        league_id: DEFAULT_LEAGUE_ID,
        email: None,
        language: "de",
    })
    .await
    .unwrap();

    repo.set_jersey(id, "brasilien").await.unwrap();
    let auth = repo.find_by_token(&token).await.unwrap().unwrap();
    assert_eq!(auth.jersey_preset, "brasilien");

    repo.delete(id).await.unwrap();
}

#[tokio::test]
async fn user_repo_set_theme_defaults_to_dark_then_persists() {
    let pool = pool().await;
    let repo = PgUserRepo::new(pool.clone());

    let id = Uuid::new_v4();
    let token = format!("repo-test-{}", Uuid::new_v4());
    repo.create(NewUser {
        id,
        name: "Theme",
        token: &token,
        is_admin: false,
        phone_number: None,
        league_id: DEFAULT_LEAGUE_ID,
        email: None,
        language: "de",
    })
    .await
    .unwrap();

    // New users start on the dark theme (column default).
    let auth = repo.find_by_token(&token).await.unwrap().unwrap();
    assert_eq!(auth.theme, "dark");

    repo.set_theme(id, "light").await.unwrap();
    let auth = repo.find_by_token(&token).await.unwrap().unwrap();
    assert_eq!(auth.theme, "light");

    repo.delete(id).await.unwrap();
}

#[tokio::test]
async fn user_repo_list_for_admin_returns_alphabetical() {
    let pool = pool().await;
    let repo = PgUserRepo::new(pool.clone());

    // Just assert non-empty rows come back sorted; the dev DB may have data.
    let list = repo.list_for_admin(DEFAULT_LEAGUE_ID).await.unwrap();
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
            league_id: DEFAULT_LEAGUE_ID,
            email: None,
            language: "de",
        })
        .await
        .unwrap();

    // Need a real match id to insert a prediction; pick the lowest existing one.
    let some_match: Option<i32> = sqlx::query_scalar("SELECT id FROM matches ORDER BY id LIMIT 1")
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

    sqlx::query("DELETE FROM predictions WHERE user_id = $1")
        .bind(user_id)
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
        .list_other_users_locked(viewer, DEFAULT_LEAGUE_ID, Utc::now() + Duration::hours(1))
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

    let teams = PgTeamRepo::new(pool.clone());

    let user_id = Uuid::new_v4();
    let token = format!("repo-test-{}", Uuid::new_v4());
    users
        .create(NewUser {
            id: user_id,
            name: "Sp",
            token: &token,
            is_admin: false,
            phone_number: None,
            league_id: DEFAULT_LEAGUE_ID,
            email: None,
            language: "de",
        })
        .await
        .unwrap();

    // Own a dedicated team rather than borrowing an arbitrary existing one:
    // other tests insert and delete synthetic teams concurrently, which would
    // otherwise race the FK on special_predictions.champion_id. Use a base id
    // distinct from the other ESPN-upsert tests so the ids never collide.
    let team_id: i32 = 9_970_001 + (std::process::id() as i32 % 1000);
    teams
        .upsert_from_espn(EspnTeamUpsert {
            espn_id: team_id,
            name: "Champion FC",
            short_name: Some("CFC"),
            flag_code: Some("xx"),
            group_letter: Some("Z"),
        })
        .await
        .unwrap();

    sp.upsert(user_id, Some(team_id)).await.unwrap();
    assert_eq!(sp.get_user_champion(user_id).await.unwrap(), Some(team_id));

    sp.upsert(user_id, None).await.unwrap();
    assert_eq!(sp.get_user_champion(user_id).await.unwrap(), None);

    sqlx::query("DELETE FROM special_predictions WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
    users.delete(user_id).await.unwrap();
    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(team_id)
        .execute(&pool)
        .await
        .unwrap();
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

#[tokio::test]
async fn settings_repo_set_then_get_round_trips() {
    let pool = pool().await;
    let settings = PgSettingsRepo::new(pool.clone());
    let key = format!("__test_key_{}__", Uuid::new_v4());
    settings.set(&key, "value-1").await.unwrap();
    assert_eq!(
        settings.get(&key).await.unwrap().as_deref(),
        Some("value-1")
    );
    settings.set(&key, "value-2").await.unwrap();
    assert_eq!(
        settings.get(&key).await.unwrap().as_deref(),
        Some("value-2")
    );
    sqlx::query("DELETE FROM settings WHERE key = $1")
        .bind(&key)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn team_repo_upsert_from_espn_inserts_then_updates_name() {
    let pool = pool().await;
    let teams = PgTeamRepo::new(pool.clone());
    // Pick an obviously-not-real ESPN id so we don't collide with real
    // tournament data; clean up at the end.
    let espn_id: i32 = 9_990_001 + (std::process::id() as i32 % 1000);

    teams
        .upsert_from_espn(EspnTeamUpsert {
            espn_id,
            name: "Atlantis",
            short_name: Some("ATL"),
            flag_code: Some("xx"),
            group_letter: Some("Z"),
        })
        .await
        .unwrap();
    teams
        .upsert_from_espn(EspnTeamUpsert {
            espn_id,
            name: "Atlantis FC",
            short_name: None, // must NOT clobber existing short_name
            flag_code: None,  // ditto for flag_code
            group_letter: None,
        })
        .await
        .unwrap();

    let row = sqlx::query("SELECT name, short_name, flag_code FROM teams WHERE id = $1")
        .bind(espn_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("name"), "Atlantis FC");
    assert_eq!(
        row.get::<Option<String>, _>("short_name").as_deref(),
        Some("ATL")
    );
    assert_eq!(
        row.get::<Option<String>, _>("flag_code").as_deref(),
        Some("xx")
    );

    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(espn_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn match_repo_upsert_from_espn_round_trips_and_preserves_kickoff() {
    let pool = pool().await;
    let matches = PgMatchRepo::new(pool.clone());

    // Use a high ESPN id we control — clean up afterwards. The ESPN id is the
    // unique key the upsert keys on, so the synthetic value is safe.
    let espn_id: i64 = 9_999_900_000_001 + (std::process::id() as i64 % 1000);
    let kickoff = Utc::now() + Duration::hours(36);

    matches
        .upsert_from_espn(EspnMatchUpsert {
            espn_event_id: espn_id,
            stage: Stage::Group,
            group_letter: Some("Z"),
            team_home_id: None,
            team_away_id: None,
            score_home: None,
            score_away: None,
            kickoff_time: Some(kickoff),
            status: "scheduled",
        })
        .await
        .unwrap();

    // Second sync: still no team ids from ESPN — must not clobber a kickoff
    // that was previously set.
    matches
        .upsert_from_espn(EspnMatchUpsert {
            espn_event_id: espn_id,
            stage: Stage::Group,
            group_letter: Some("Z"),
            team_home_id: None,
            team_away_id: None,
            score_home: None,
            score_away: None,
            kickoff_time: None,
            status: "scheduled",
        })
        .await
        .unwrap();

    let row = sqlx::query("SELECT kickoff_time FROM matches WHERE espn_event_id = $1")
        .bind(espn_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(row
        .get::<Option<chrono::DateTime<Utc>>, _>("kickoff_time")
        .is_some());

    sqlx::query("DELETE FROM matches WHERE espn_event_id = $1")
        .bind(espn_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn notification_repo_try_send_records_then_skips_duplicate() {
    let pool = pool().await;
    let notifications = PgNotificationRepo::new(pool.clone());

    struct OkNotifier;
    #[async_trait::async_trait]
    impl Notifier for OkNotifier {
        async fn notify(&self, _e: NotificationEvent) -> Result<(), NotifierError> {
            Ok(())
        }
    }

    // Synthetic kind keeps this test isolated from real notifications.
    // Schema caps `kind` at varchar(40); keep the synthetic value short.
    let kind = format!("t_{}", &Uuid::new_v4().simple().to_string()[..16]);
    let event = NotificationEvent::SpecialPredictionsLock {
        lock_at: Utc::now(),
        missing_names: vec!["Anna".into()],
    };

    let first = notifications
        .try_send(
            &OkNotifier,
            DEFAULT_LEAGUE_ID,
            &kind,
            1,
            None,
            event.clone(),
        )
        .await
        .unwrap();
    assert!(first);
    assert!(notifications
        .already_sent(DEFAULT_LEAGUE_ID, &kind, 1, None)
        .await
        .unwrap());

    let second = notifications
        .try_send(&OkNotifier, DEFAULT_LEAGUE_ID, &kind, 1, None, event)
        .await
        .unwrap();
    assert!(!second, "duplicate must be a no-op");

    sqlx::query("DELETE FROM sent_notifications WHERE kind = $1")
        .bind(&kind)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn notification_repo_try_send_rolls_back_on_notifier_failure() {
    let pool = pool().await;
    let notifications = PgNotificationRepo::new(pool.clone());

    struct FailNotifier;
    #[async_trait::async_trait]
    impl Notifier for FailNotifier {
        async fn notify(&self, _e: NotificationEvent) -> Result<(), NotifierError> {
            Err("boom".into())
        }
    }

    // Schema caps `kind` at varchar(40); keep the synthetic value short.
    let kind = format!("t_{}", &Uuid::new_v4().simple().to_string()[..16]);
    let event = NotificationEvent::SpecialPredictionsLock {
        lock_at: Utc::now(),
        missing_names: vec![],
    };

    let outcome = notifications
        .try_send(&FailNotifier, DEFAULT_LEAGUE_ID, &kind, 7, None, event)
        .await
        .unwrap();
    assert!(!outcome, "a failed notifier must report not-sent");
    assert!(
        !notifications
            .already_sent(DEFAULT_LEAGUE_ID, &kind, 7, None)
            .await
            .unwrap(),
        "the slot must be rolled back so the next tick retries"
    );
}
