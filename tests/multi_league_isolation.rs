//! Multi-league tenant-isolation tests.
//!
//! These exercise the in-memory fakes — every aggregate query that powers
//! the leaderboard, badges, "other tips" panel, admin user list, and the
//! notification idempotency table must filter by league. A regression here
//! would let a user in League A see (or be ranked against) data from
//! League B, undermining the whole multi-tenancy promise.
//!
//! Each test sets up two leagues with overlapping data and asserts that
//! every public read returns only the rows belonging to the queried league.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use pila::badges;
use pila::handlers::services::{build_badge_context, fetch_leaderboard};
use pila::repo::league::{League, MemoryLeagueRepo};
use pila::repo::prediction::{FakeFinishedRow, FakeLeaderboardRow};
use pila::repo::user::{NewUser, UserFull};
use pila::repo::{
    MemoryBootstrapRepo, MemoryInviteRepo, MemoryMatchRepo, MemoryNotificationRepo,
    MemoryPredictionRepo, MemorySettingsRepo, MemorySpecialPredictionRepo, MemoryTeamRepo,
    MemoryUserRepo, Repos, DEFAULT_LEAGUE_ID,
};
use pila::stage::Stage;

// ─── Setup helpers ───────────────────────────────────────────────────────────

struct Two {
    repos: Repos,
    /// Concrete handles to the in-memory repos so tests can seed rows the
    /// trait doesn't expose (`seed_finished`, `seed_user`, etc.). The
    /// `_special_predictions` handle is held to keep its Arc alive even
    /// though tests don't call its setters after `setup_two_leagues` returns.
    predictions: Arc<MemoryPredictionRepo>,
    _special_predictions: Arc<MemorySpecialPredictionRepo>,
    notifications: Arc<MemoryNotificationRepo>,
    league_a: Uuid,
    league_b: Uuid,
    /// Liga-A members.
    a_alice: Uuid,
    a_bob: Uuid,
    /// Liga-B members.
    b_charlie: Uuid,
}

fn user_full(id: Uuid, league_id: Uuid, name: &str, token: &str) -> UserFull {
    UserFull {
        id,
        name: name.into(),
        real_name: name.into(),
        token: token.into(),
        phone_number: None,
        email: None,
        is_admin: false,
        can_create_league: false,
        league_id,
        language: "de".into(),
    }
}

/// Build two leagues. League A has Alice + Bob, League B has Charlie.
/// Both leagues are seeded into every repo that needs to know about them.
fn setup_two_leagues() -> Two {
    let users = Arc::new(MemoryUserRepo::new());
    let leagues = Arc::new(MemoryLeagueRepo::new());
    let predictions = Arc::new(MemoryPredictionRepo::new());
    let special_predictions = Arc::new(MemorySpecialPredictionRepo::new());
    let notifications = Arc::new(MemoryNotificationRepo::new());

    let league_a = DEFAULT_LEAGUE_ID;
    let league_b = Uuid::new_v4();
    leagues.seed(League {
        id: league_a,
        name: "Liga A".into(),
        notifications_bootstrapped: true,
    });
    leagues.seed(League {
        id: league_b,
        name: "Liga B".into(),
        notifications_bootstrapped: false,
    });

    let a_alice = Uuid::new_v4();
    let a_bob = Uuid::new_v4();
    let b_charlie = Uuid::new_v4();

    users.seed(user_full(a_alice, league_a, "Alice", "tk-alice"), "classic");
    users.seed(user_full(a_bob, league_a, "Bob", "tk-bob"), "classic");
    users.seed(
        user_full(b_charlie, league_b, "Charlie", "tk-charlie"),
        "classic",
    );

    // Notification fake also needs to know about each user's league.
    notifications.seed_user(league_a, "Alice");
    notifications.seed_user(league_a, "Bob");
    notifications.seed_user(league_b, "Charlie");

    // Special-pick fake stores its own user→league mapping.
    special_predictions.seed_user(a_alice, league_a, "Alice");
    special_predictions.seed_user(a_bob, league_a, "Bob");
    special_predictions.seed_user(b_charlie, league_b, "Charlie");

    let repos = Repos {
        bootstrap: Arc::new(MemoryBootstrapRepo::new()),
        users,
        leagues,
        matches: Arc::new(MemoryMatchRepo::new()),
        predictions: predictions.clone(),
        special_predictions: special_predictions.clone(),
        teams: Arc::new(MemoryTeamRepo::new()),
        settings: Arc::new(MemorySettingsRepo::new()),
        invites: Arc::new(MemoryInviteRepo::new()),
        notifications: notifications.clone(),
    };

    Two {
        repos,
        predictions,
        _special_predictions: special_predictions,
        notifications,
        league_a,
        league_b,
        a_alice,
        a_bob,
        b_charlie,
    }
}

fn jerseys() -> std::collections::HashMap<String, pila::jersey::JerseyPreset> {
    pila::jersey::load().as_ref().clone()
}

fn finished_pred(
    user_id: Uuid,
    league_id: Uuid,
    match_id: i32,
    score: (i32, i32),
    pred: (i32, i32),
) -> FakeFinishedRow {
    FakeFinishedRow {
        user_id,
        league_id,
        match_id,
        stage: Stage::Group,
        kickoff: Utc::now() - chrono::Duration::days(1),
        score_home: score.0,
        score_away: score.1,
        predicted_home: pred.0,
        predicted_away: pred.1,
    }
}

// ─── User-repo isolation ─────────────────────────────────────────────────────

#[tokio::test]
async fn list_for_admin_only_returns_league_members() {
    let t = setup_two_leagues();
    let a = t.repos.users.list_for_admin(t.league_a).await.unwrap();
    let names_a: Vec<&str> = a.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names_a, vec!["Alice", "Bob"], "Liga A members only");

    let b = t.repos.users.list_for_admin(t.league_b).await.unwrap();
    let names_b: Vec<&str> = b.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names_b, vec!["Charlie"], "Liga B members only");
}

#[tokio::test]
async fn list_basic_only_returns_league_members() {
    let t = setup_two_leagues();
    let a = t.repos.users.list_basic(t.league_a).await.unwrap();
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|u| u.name == "Alice" || u.name == "Bob"));

    let b = t.repos.users.list_basic(t.league_b).await.unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].name, "Charlie");
}

#[tokio::test]
async fn list_ids_only_returns_league_members() {
    let t = setup_two_leagues();
    let ids_a = t.repos.users.list_ids(t.league_a).await.unwrap();
    assert!(ids_a.contains(&t.a_alice));
    assert!(ids_a.contains(&t.a_bob));
    assert!(!ids_a.contains(&t.b_charlie), "Liga B user must not leak");

    let ids_b = t.repos.users.list_ids(t.league_b).await.unwrap();
    assert!(ids_b.contains(&t.b_charlie));
    assert!(!ids_b.contains(&t.a_alice));
}

#[tokio::test]
async fn list_all_ids_returns_every_user_across_leagues() {
    let t = setup_two_leagues();
    let all = t.repos.users.list_all_ids().await.unwrap();
    assert_eq!(all.len(), 3, "global view spans both leagues");
}

// ─── Prediction-repo isolation ───────────────────────────────────────────────

#[tokio::test]
async fn finished_join_only_returns_predictions_from_queried_league() {
    let t = setup_two_leagues();

    t.predictions
        .seed_finished(finished_pred(t.a_alice, t.league_a, 1, (2, 1), (2, 1)));
    t.predictions
        .seed_finished(finished_pred(t.b_charlie, t.league_b, 1, (2, 1), (2, 1)));

    let a_rows = t
        .repos
        .predictions
        .list_finished_join(t.league_a)
        .await
        .unwrap();
    assert_eq!(a_rows.len(), 1);
    assert_eq!(a_rows[0].user_id, t.a_alice);

    let b_rows = t
        .repos
        .predictions
        .list_finished_join(t.league_b)
        .await
        .unwrap();
    assert_eq!(b_rows.len(), 1);
    assert_eq!(b_rows[0].user_id, t.b_charlie);
}

#[tokio::test]
async fn leaderboard_join_only_returns_users_from_queried_league() {
    let t = setup_two_leagues();

    t.predictions.seed_leaderboard(FakeLeaderboardRow {
        league_id: t.league_a,
        user_name: "Alice".into(),
        stage: Stage::Group,
        kickoff_time: Some(Utc::now() - chrono::Duration::hours(2)),
        status: "finished".into(),
        score_home: Some(2),
        score_away: Some(1),
        predicted_home: 2,
        predicted_away: 1,
    });
    t.predictions.seed_leaderboard(FakeLeaderboardRow {
        league_id: t.league_b,
        user_name: "Charlie".into(),
        stage: Stage::Group,
        kickoff_time: Some(Utc::now() - chrono::Duration::hours(2)),
        status: "finished".into(),
        score_home: Some(0),
        score_away: Some(0),
        predicted_home: 0,
        predicted_away: 0,
    });

    let a = t
        .repos
        .predictions
        .list_leaderboard_join(t.league_a)
        .await
        .unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].user_name, "Alice");

    let b = t
        .repos
        .predictions
        .list_leaderboard_join(t.league_b)
        .await
        .unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].user_name, "Charlie");
}

// ─── Special-prediction isolation ────────────────────────────────────────────

#[tokio::test]
async fn list_all_picks_only_returns_picks_from_queried_league() {
    let t = setup_two_leagues();
    t.repos
        .special_predictions
        .upsert(t.a_alice, Some(11))
        .await
        .unwrap();
    t.repos
        .special_predictions
        .upsert(t.b_charlie, Some(22))
        .await
        .unwrap();

    let a = t
        .repos
        .special_predictions
        .list_all_picks(t.league_a)
        .await
        .unwrap();
    assert_eq!(a, vec![(t.a_alice, 11)]);

    let b = t
        .repos
        .special_predictions
        .list_all_picks(t.league_b)
        .await
        .unwrap();
    assert_eq!(b, vec![(t.b_charlie, 22)]);
}

#[tokio::test]
async fn list_with_user_names_only_returns_picks_from_queried_league() {
    let t = setup_two_leagues();
    t.repos
        .special_predictions
        .upsert(t.a_alice, Some(11))
        .await
        .unwrap();
    t.repos
        .special_predictions
        .upsert(t.b_charlie, Some(22))
        .await
        .unwrap();

    let a = t
        .repos
        .special_predictions
        .list_with_user_names(t.league_a)
        .await
        .unwrap();
    let names: Vec<&str> = a.iter().map(|r| r.user_name.as_str()).collect();
    assert!(names.contains(&"Alice"));
    assert!(!names.contains(&"Charlie"));
}

// ─── Notification-repo isolation ─────────────────────────────────────────────

#[tokio::test]
async fn users_missing_prediction_only_returns_league_members() {
    let t = setup_two_leagues();
    // Alice has tipped match 5 — Bob has not. Charlie (Liga B) has not either.
    t.notifications.seed_prediction(t.league_a, "Alice", 5);

    let missing_a = t
        .repos
        .notifications
        .users_missing_prediction_for(t.league_a, 5)
        .await
        .unwrap();
    assert_eq!(missing_a, vec!["Bob".to_string()]);

    let missing_b = t
        .repos
        .notifications
        .users_missing_prediction_for(t.league_b, 5)
        .await
        .unwrap();
    assert_eq!(
        missing_b,
        vec!["Charlie".to_string()],
        "Liga B's missing-tippers list must not contain Bob from Liga A"
    );
}

#[tokio::test]
async fn users_missing_champion_only_returns_league_members() {
    let t = setup_two_leagues();

    let missing_a = t
        .repos
        .notifications
        .users_missing_champion(t.league_a)
        .await
        .unwrap();
    assert_eq!(
        missing_a,
        vec!["Alice".to_string(), "Bob".to_string()],
        "Liga A: both members lack a champion pick (none seeded)"
    );

    let missing_b = t
        .repos
        .notifications
        .users_missing_champion(t.league_b)
        .await
        .unwrap();
    assert_eq!(missing_b, vec!["Charlie".to_string()]);
}

#[tokio::test]
async fn notification_idempotency_is_per_league() {
    let t = setup_two_leagues();

    // Liga A records (match_closing_soon, 5).
    let recorded = t
        .repos
        .notifications
        .try_send(
            &OkNotifier,
            t.league_a,
            "match_closing_soon",
            5,
            None,
            sample_event(),
        )
        .await
        .unwrap();
    assert!(recorded);

    // Liga A second attempt is a no-op.
    let again = t
        .repos
        .notifications
        .try_send(
            &OkNotifier,
            t.league_a,
            "match_closing_soon",
            5,
            None,
            sample_event(),
        )
        .await
        .unwrap();
    assert!(
        !again,
        "second send for same (league, kind, ref) is a no-op"
    );

    // Liga B's slot for the same (kind, ref) is independent — must succeed.
    let b_first = t
        .repos
        .notifications
        .try_send(
            &OkNotifier,
            t.league_b,
            "match_closing_soon",
            5,
            None,
            sample_event(),
        )
        .await
        .unwrap();
    assert!(
        b_first,
        "Liga B is independent — Liga A's send must not block it"
    );
}

#[tokio::test]
async fn silence_existing_matches_only_marks_one_league() {
    let t = setup_two_leagues();
    t.notifications.seed_match_with_both_teams(1);
    t.notifications.seed_match_with_both_teams(2);

    t.repos
        .notifications
        .silence_existing_matches(t.league_a)
        .await
        .unwrap();

    assert!(t
        .repos
        .notifications
        .already_sent(t.league_a, "match_closing_soon", 1, None)
        .await
        .unwrap());
    assert!(!t
        .repos
        .notifications
        .already_sent(t.league_b, "match_closing_soon", 1, None)
        .await
        .unwrap());
}

// ─── League-config isolation ─────────────────────────────────────────────────

#[tokio::test]
async fn league_settings_are_isolated_between_leagues() {
    let t = setup_two_leagues();
    t.repos
        .leagues
        .set_setting(
            t.league_a,
            pila::repo::LeagueConfig::KEY_SIGNAL_GROUP_ID,
            Some("group.A"),
        )
        .await
        .unwrap();
    t.repos
        .leagues
        .set_setting(
            t.league_a,
            pila::repo::LeagueConfig::KEY_DEFAULT_LANGUAGE,
            Some("en"),
        )
        .await
        .unwrap();

    let cfg_a = t.repos.leagues.get_config(t.league_a).await.unwrap();
    assert_eq!(cfg_a.signal_group_id.as_deref(), Some("group.A"));
    assert_eq!(cfg_a.default_language, "en");

    let cfg_b = t.repos.leagues.get_config(t.league_b).await.unwrap();
    assert!(
        cfg_b.signal_group_id.is_none(),
        "League B's signal config must be untouched"
    );
    assert_eq!(
        cfg_b.default_language, "de",
        "League B falls back to default language"
    );
}

// ─── Service-layer isolation (badges + leaderboard composition) ──────────────

#[tokio::test]
async fn badge_context_finished_predictions_only_for_queried_league() {
    let t = setup_two_leagues();
    t.predictions
        .seed_finished(finished_pred(t.a_alice, t.league_a, 1, (2, 1), (2, 1)));
    t.predictions
        .seed_finished(finished_pred(t.b_charlie, t.league_b, 1, (2, 1), (2, 1)));

    let ctx_a = build_badge_context(&t.repos, t.a_alice, t.league_a, Utc::now()).await;
    assert_eq!(ctx_a.finished_predictions.len(), 1);
    assert_eq!(ctx_a.finished_predictions[0].user_id, t.a_alice);

    let ctx_b = build_badge_context(&t.repos, t.b_charlie, t.league_b, Utc::now()).await;
    assert_eq!(ctx_b.finished_predictions.len(), 1);
    assert_eq!(ctx_b.finished_predictions[0].user_id, t.b_charlie);
}

#[tokio::test]
async fn badge_context_all_user_ids_only_for_queried_league() {
    let t = setup_two_leagues();
    let ctx_a = build_badge_context(&t.repos, t.a_alice, t.league_a, Utc::now()).await;
    assert_eq!(ctx_a.all_user_ids.len(), 2);
    assert!(ctx_a.all_user_ids.contains(&t.a_alice));
    assert!(ctx_a.all_user_ids.contains(&t.a_bob));
    assert!(!ctx_a.all_user_ids.contains(&t.b_charlie));

    let ctx_b = build_badge_context(&t.repos, t.b_charlie, t.league_b, Utc::now()).await;
    assert_eq!(ctx_b.all_user_ids, vec![t.b_charlie]);
}

#[tokio::test]
async fn badges_compute_only_within_one_league() {
    let t = setup_two_leagues();
    // Alice gets a perfect tip in Liga A; Charlie gets a perfect tip in Liga B
    // for the same (synthetic) match. Each league should see exactly one
    // exact-result badge counted for its own member.
    t.predictions
        .seed_finished(finished_pred(t.a_alice, t.league_a, 1, (2, 1), (2, 1)));
    t.predictions
        .seed_finished(finished_pred(t.b_charlie, t.league_b, 1, (2, 1), (2, 1)));

    let ctx_a_owned = build_badge_context(&t.repos, t.a_alice, t.league_a, Utc::now()).await;
    let badges_a = badges::compute_all(&ctx_a_owned.as_ctx(), &pila::translations::T::default());
    let exact_a = badges_a
        .iter()
        .find(|b| b.key == "exact_count")
        .expect("exact-count badge present");
    assert_eq!(exact_a.display.times_earned(), 1, "Alice has 1 exact tip");

    // For Bob (Liga A), Charlie's perfect tip must not count.
    let ctx_bob_owned = build_badge_context(&t.repos, t.a_bob, t.league_a, Utc::now()).await;
    let badges_bob =
        badges::compute_all(&ctx_bob_owned.as_ctx(), &pila::translations::T::default());
    let exact_bob = badges_bob.iter().find(|b| b.key == "exact_count").unwrap();
    assert_eq!(
        exact_bob.display.times_earned(),
        0,
        "Bob has none of his own exact tips, Charlie's must not leak"
    );
}

#[tokio::test]
async fn admin_user_list_only_returns_league_members() {
    let t = setup_two_leagues();
    let rows_a = t.repos.users.list_for_admin(t.league_a).await.unwrap();
    let names_a: Vec<&str> = rows_a.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names_a, vec!["Alice", "Bob"]);

    let rows_b = t.repos.users.list_for_admin(t.league_b).await.unwrap();
    let names_b: Vec<&str> = rows_b.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names_b, vec!["Charlie"]);
}

#[tokio::test]
async fn fetch_leaderboard_only_returns_league_members() {
    let t = setup_two_leagues();
    t.predictions.seed_leaderboard(FakeLeaderboardRow {
        league_id: t.league_a,
        user_name: "Alice".into(),
        stage: Stage::Group,
        kickoff_time: Some(Utc::now() - chrono::Duration::hours(2)),
        status: "finished".into(),
        score_home: Some(2),
        score_away: Some(1),
        predicted_home: 2,
        predicted_away: 1,
    });
    t.predictions.seed_leaderboard(FakeLeaderboardRow {
        league_id: t.league_b,
        user_name: "Charlie".into(),
        stage: Stage::Group,
        kickoff_time: Some(Utc::now() - chrono::Duration::hours(2)),
        status: "finished".into(),
        score_home: Some(2),
        score_away: Some(1),
        predicted_home: 2,
        predicted_away: 1,
    });

    let lb_a = fetch_leaderboard(&t.repos, &jerseys(), t.league_a, Utc::now()).await;
    let names_a: Vec<&str> = lb_a.iter().map(|e| e.name.as_str()).collect();
    assert!(names_a.contains(&"Alice"));
    assert!(names_a.contains(&"Bob"));
    assert!(
        !names_a.contains(&"Charlie"),
        "Charlie is in Liga B and must not appear in Liga A's leaderboard"
    );

    let lb_b = fetch_leaderboard(&t.repos, &jerseys(), t.league_b, Utc::now()).await;
    let names_b: Vec<&str> = lb_b.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names_b, vec!["Charlie"]);
}

// ─── New-user invite ends up in correct league ───────────────────────────────

#[tokio::test]
async fn new_user_create_lands_in_specified_league() {
    let t = setup_two_leagues();
    let new_id = Uuid::new_v4();
    t.repos
        .users
        .create(NewUser {
            id: new_id,
            name: "Newcomer",
            real_name: "Newcomer",
            token: "tk-new",
            is_admin: false,
            phone_number: None,
            league_id: t.league_b,
            language: "de",
            email: None,
        })
        .await
        .unwrap();

    let ids_b = t.repos.users.list_ids(t.league_b).await.unwrap();
    assert!(ids_b.contains(&new_id));
    let ids_a = t.repos.users.list_ids(t.league_a).await.unwrap();
    assert!(!ids_a.contains(&new_id));
}

// ─── Sample helpers ──────────────────────────────────────────────────────────

struct OkNotifier;
#[async_trait::async_trait]
impl pila::notifier::Notifier for OkNotifier {
    async fn notify(
        &self,
        _event: pila::notifier::NotificationEvent,
    ) -> Result<(), pila::notifier::NotifierError> {
        Ok(())
    }
}

fn sample_event() -> pila::notifier::NotificationEvent {
    pila::notifier::NotificationEvent::SpecialPredictionsLock {
        lock_at: Utc::now(),
        missing_names: vec!["Anna".into()],
    }
}
