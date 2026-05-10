//! Sent-notification idempotency persistence + the dispatch tx used by
//! the worker.
//!
//! The `try_send` method intentionally couples the trait to the `Notifier`
//! abstraction: idempotent dispatch is a single-transaction operation
//! (insert sentinel row → run notifier → commit-or-rollback) that should
//! not be split across the repo/worker boundary, otherwise a process
//! crash between phases leaves a phantom claim that prevents retry.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use super::{RepoError, RepoResult};
use crate::notifier::{NotificationEvent, Notifier};
use crate::stage::Stage;

/// One match that's about to lock and still has at least one missing tip.
#[derive(Debug, Clone)]
pub struct ClosingSoonMatch {
    pub match_id: i32,
    pub stage: Stage,
    pub group_letter: Option<String>,
    pub kickoff_time: DateTime<Utc>,
    pub home: String,
    pub away: String,
}

#[async_trait]
pub trait NotificationRepo: Send + Sync {
    /// True if the bootstrap-silence step has already run.
    async fn was_bootstrapped(&self) -> RepoResult<bool>;

    /// Mark the bootstrap step as done.
    async fn mark_bootstrapped(&self) -> RepoResult<()>;

    /// Insert a sentinel `match_closing_soon` row for every currently-known
    /// fixture so the very first worker tick after a fresh deploy does not
    /// flood the group with retroactive reminders. Idempotent on
    /// `(kind, ref_id)`.
    async fn silence_existing_matches(&self) -> RepoResult<()>;

    /// Matches with a kickoff in the next 24h that have not yet had a
    /// `match_closing_soon` row recorded.
    async fn list_closing_soon_unnotified(&self) -> RepoResult<Vec<ClosingSoonMatch>>;

    /// Names of users without a tip on the given match. Sorted alphabetically.
    async fn users_missing_prediction_for(&self, match_id: i32) -> RepoResult<Vec<String>>;

    /// Names of users without a champion pick. Sorted alphabetically.
    async fn users_missing_champion(&self) -> RepoResult<Vec<String>>;

    /// Whether `(kind, ref_id)` has already been recorded as sent.
    async fn already_sent(&self, kind: &str, ref_id: i32) -> RepoResult<bool>;

    /// Atomic dispatch primitive. In one transaction:
    ///   1. INSERT into `sent_notifications` ON CONFLICT DO NOTHING.
    ///   2. If the insert was a no-op (already recorded), rollback and
    ///      return `false`.
    ///   3. Otherwise call `notifier.notify(event)`. On success commit and
    ///      return `true`. On failure rollback (so the next worker tick
    ///      retries) and return `false`.
    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        kind: &str,
        ref_id: i32,
        event: NotificationEvent,
    ) -> RepoResult<bool>;
}

// ─── Postgres implementation ─────────────────────────────────────────────────

pub struct PgNotificationRepo {
    pool: PgPool,
}

impl PgNotificationRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NotificationRepo for PgNotificationRepo {
    async fn was_bootstrapped(&self) -> RepoResult<bool> {
        let row = sqlx::query!(
            "SELECT 1 AS dummy FROM settings WHERE key = 'notifications_bootstrapped' AND value = 'true'"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row.is_some())
    }

    async fn mark_bootstrapped(&self) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO settings (key, value) VALUES ('notifications_bootstrapped', 'true') \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value"
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn silence_existing_matches(&self) -> RepoResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO sent_notifications (kind, ref_id)
            SELECT 'match_closing_soon', m.id FROM matches m
            WHERE m.team_home_id IS NOT NULL AND m.team_away_id IS NOT NULL
            ON CONFLICT (kind, ref_id) DO NOTHING
            "#
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn list_closing_soon_unnotified(&self) -> RepoResult<Vec<ClosingSoonMatch>> {
        let rows = sqlx::query!(
            r#"
            SELECT m.id,
                   m.stage AS "stage: Stage",
                   m.group_letter,
                   m.kickoff_time AS "kickoff_time!",
                   th.name AS home,
                   ta.name AS away
            FROM matches m
            JOIN teams th ON th.id = m.team_home_id
            JOIN teams ta ON ta.id = m.team_away_id
            LEFT JOIN sent_notifications n
              ON n.kind = 'match_closing_soon' AND n.ref_id = m.id
            WHERE m.team_home_id IS NOT NULL AND m.team_away_id IS NOT NULL
              AND m.kickoff_time IS NOT NULL
              AND m.kickoff_time BETWEEN NOW() AND NOW() + INTERVAL '24 hours'
              AND n.ref_id IS NULL
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| ClosingSoonMatch {
                match_id: r.id,
                stage: r.stage,
                group_letter: r.group_letter,
                kickoff_time: r.kickoff_time,
                home: r.home,
                away: r.away,
            })
            .collect())
    }

    async fn users_missing_prediction_for(&self, match_id: i32) -> RepoResult<Vec<String>> {
        let names = sqlx::query_scalar!(
            r#"
            SELECT u.name FROM users u
            WHERE NOT EXISTS (
                SELECT 1 FROM predictions p
                WHERE p.user_id = u.id AND p.match_id = $1
            )
            ORDER BY u.name
            "#,
            match_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(names)
    }

    async fn users_missing_champion(&self) -> RepoResult<Vec<String>> {
        let names = sqlx::query_scalar!(
            r#"
            SELECT u.name FROM users u
            WHERE NOT EXISTS (
                SELECT 1 FROM special_predictions sp
                WHERE sp.user_id = u.id AND sp.champion_id IS NOT NULL
            )
            ORDER BY u.name
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(names)
    }

    async fn already_sent(&self, kind: &str, ref_id: i32) -> RepoResult<bool> {
        let row = sqlx::query!(
            "SELECT 1 AS dummy FROM sent_notifications WHERE kind = $1 AND ref_id = $2",
            kind,
            ref_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row.is_some())
    }

    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        kind: &str,
        ref_id: i32,
        event: NotificationEvent,
    ) -> RepoResult<bool> {
        let mut tx = self.pool.begin().await.map_err(RepoError::from)?;

        let inserted = sqlx::query_scalar!(
            "INSERT INTO sent_notifications (kind, ref_id) VALUES ($1, $2)
             ON CONFLICT (kind, ref_id) DO NOTHING
             RETURNING ref_id",
            kind,
            ref_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(RepoError::from)?
        .is_some();

        if !inserted {
            // Already-sent path: drop the (effectively no-op) tx.
            let _ = tx.rollback().await;
            return Ok(false);
        }

        match notifier.notify(event).await {
            Ok(()) => {
                tx.commit().await.map_err(RepoError::from)?;
                tracing::info!("Sent notification: {} {}", kind, ref_id);
                Ok(true)
            }
            Err(e) => {
                tracing::error!("Notifier failed for {} {}: {:?}", kind, ref_id, e);
                let _ = tx.rollback().await;
                Ok(false)
            }
        }
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

#[derive(Default)]
struct MemoryNotificationState {
    bootstrapped: bool,
    sent: HashSet<(String, i32)>,
    /// Matches indexed by id. Test code seeds this so the queries have rows
    /// to find. Keeps the fake decoupled from `MemoryMatchRepo`.
    closing_soon: Vec<ClosingSoonMatch>,
    /// User → set of match_ids they have tipped.
    predictions: HashMap<String, HashSet<i32>>,
    /// All registered user names.
    user_names: Vec<String>,
    /// Names of users with a champion pick.
    champion_picked: HashSet<String>,
    /// All currently-known match ids (used by silence_existing_matches).
    matches_with_both_teams: Vec<i32>,
}

#[derive(Default)]
pub struct MemoryNotificationRepo {
    state: Mutex<MemoryNotificationState>,
}

impl MemoryNotificationRepo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed_user(&self, name: &str) {
        let mut s = self.state.lock().unwrap();
        s.user_names.push(name.to_string());
    }

    pub fn seed_user_with_champion(&self, name: &str) {
        let mut s = self.state.lock().unwrap();
        s.user_names.push(name.to_string());
        s.champion_picked.insert(name.to_string());
    }

    pub fn seed_prediction(&self, user_name: &str, match_id: i32) {
        let mut s = self.state.lock().unwrap();
        s.predictions
            .entry(user_name.to_string())
            .or_default()
            .insert(match_id);
    }

    pub fn seed_closing_soon(&self, m: ClosingSoonMatch) {
        let mut s = self.state.lock().unwrap();
        s.matches_with_both_teams.push(m.match_id);
        s.closing_soon.push(m);
    }

    pub fn seed_match_with_both_teams(&self, match_id: i32) {
        self.state
            .lock()
            .unwrap()
            .matches_with_both_teams
            .push(match_id);
    }

    pub fn was_bootstrapped_sync(&self) -> bool {
        self.state.lock().unwrap().bootstrapped
    }

    pub fn sent_count(&self) -> usize {
        self.state.lock().unwrap().sent.len()
    }
}

#[async_trait]
impl NotificationRepo for MemoryNotificationRepo {
    async fn was_bootstrapped(&self) -> RepoResult<bool> {
        Ok(self.state.lock().unwrap().bootstrapped)
    }

    async fn mark_bootstrapped(&self) -> RepoResult<()> {
        self.state.lock().unwrap().bootstrapped = true;
        Ok(())
    }

    async fn silence_existing_matches(&self) -> RepoResult<()> {
        let mut s = self.state.lock().unwrap();
        let ids: Vec<i32> = s.matches_with_both_teams.clone();
        for id in ids {
            s.sent.insert(("match_closing_soon".into(), id));
        }
        Ok(())
    }

    async fn list_closing_soon_unnotified(&self) -> RepoResult<Vec<ClosingSoonMatch>> {
        let s = self.state.lock().unwrap();
        Ok(s.closing_soon
            .iter()
            .filter(|m| !s.sent.contains(&("match_closing_soon".into(), m.match_id)))
            .cloned()
            .collect())
    }

    async fn users_missing_prediction_for(&self, match_id: i32) -> RepoResult<Vec<String>> {
        let s = self.state.lock().unwrap();
        let mut names: Vec<String> = s
            .user_names
            .iter()
            .filter(|n| {
                !s.predictions
                    .get(*n)
                    .is_some_and(|m| m.contains(&match_id))
            })
            .cloned()
            .collect();
        names.sort();
        Ok(names)
    }

    async fn users_missing_champion(&self) -> RepoResult<Vec<String>> {
        let s = self.state.lock().unwrap();
        let mut names: Vec<String> = s
            .user_names
            .iter()
            .filter(|n| !s.champion_picked.contains(*n))
            .cloned()
            .collect();
        names.sort();
        Ok(names)
    }

    async fn already_sent(&self, kind: &str, ref_id: i32) -> RepoResult<bool> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .sent
            .contains(&(kind.to_string(), ref_id)))
    }

    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        kind: &str,
        ref_id: i32,
        event: NotificationEvent,
    ) -> RepoResult<bool> {
        // Two-phase emulation of the Pg tx semantics: tentatively reserve the
        // slot, run the notifier, commit on success and release on failure.
        {
            let mut s = self.state.lock().unwrap();
            if !s.sent.insert((kind.to_string(), ref_id)) {
                return Ok(false);
            }
        }

        match notifier.notify(event).await {
            Ok(()) => Ok(true),
            Err(_) => {
                // Roll back the tentative reservation so the next tick retries.
                self.state
                    .lock()
                    .unwrap()
                    .sent
                    .remove(&(kind.to_string(), ref_id));
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::notifier::NotifierError;

    /// Drop-target fake notifier that always succeeds.
    struct OkNotifier;
    #[async_trait]
    impl Notifier for OkNotifier {
        async fn notify(&self, _event: NotificationEvent) -> Result<(), NotifierError> {
            Ok(())
        }
    }

    /// Always-fail notifier — used to assert the slot is released on error.
    struct FailNotifier;
    #[async_trait]
    impl Notifier for FailNotifier {
        async fn notify(&self, _event: NotificationEvent) -> Result<(), NotifierError> {
            Err("nope".into())
        }
    }

    fn lock_event() -> NotificationEvent {
        NotificationEvent::SpecialPredictionsLock {
            lock_at: Utc::now(),
            missing_names: vec!["Anna".into()],
        }
    }

    #[tokio::test]
    async fn try_send_succeeds_first_time_and_records_slot() {
        let repo = MemoryNotificationRepo::new();
        let sent = repo
            .try_send(&OkNotifier, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();
        assert!(sent);
        assert!(repo.already_sent("match_closing_soon", 1).await.unwrap());
    }

    #[tokio::test]
    async fn try_send_skips_already_recorded_slot() {
        let repo = MemoryNotificationRepo::new();
        let _ = repo
            .try_send(&OkNotifier, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();
        let sent_again = repo
            .try_send(&OkNotifier, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();
        assert!(!sent_again, "second call must be a no-op");
        assert_eq!(repo.sent_count(), 1);
    }

    #[tokio::test]
    async fn try_send_releases_slot_on_notifier_failure() {
        let repo = MemoryNotificationRepo::new();
        let sent = repo
            .try_send(&FailNotifier, "special_lock_soon", 0, lock_event())
            .await
            .unwrap();
        assert!(!sent);
        assert!(
            !repo.already_sent("special_lock_soon", 0).await.unwrap(),
            "slot must be released so the next tick retries"
        );
    }

    #[tokio::test]
    async fn bootstrap_silences_known_matches_only_once() {
        let repo = MemoryNotificationRepo::new();
        repo.seed_match_with_both_teams(1);
        repo.seed_match_with_both_teams(2);
        assert!(!repo.was_bootstrapped().await.unwrap());

        repo.silence_existing_matches().await.unwrap();
        repo.mark_bootstrapped().await.unwrap();

        assert!(repo.was_bootstrapped().await.unwrap());
        assert!(repo.already_sent("match_closing_soon", 1).await.unwrap());
        assert!(repo.already_sent("match_closing_soon", 2).await.unwrap());
    }

    #[tokio::test]
    async fn users_missing_prediction_excludes_those_who_tipped() {
        let repo = MemoryNotificationRepo::new();
        repo.seed_user("Anna");
        repo.seed_user("Ben");
        repo.seed_user("Cleo");
        repo.seed_prediction("Ben", 5);

        let missing = repo.users_missing_prediction_for(5).await.unwrap();
        assert_eq!(missing, vec!["Anna".to_string(), "Cleo".to_string()]);
    }

    #[tokio::test]
    async fn users_missing_champion_excludes_pickers() {
        let repo = MemoryNotificationRepo::new();
        repo.seed_user("Anna");
        repo.seed_user_with_champion("Ben");
        let missing = repo.users_missing_champion().await.unwrap();
        assert_eq!(missing, vec!["Anna".to_string()]);
    }

    #[tokio::test]
    async fn list_closing_soon_excludes_already_sent() {
        let repo = MemoryNotificationRepo::new();
        repo.seed_closing_soon(ClosingSoonMatch {
            match_id: 1,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            kickoff_time: Utc::now() + chrono::Duration::hours(2),
            home: "GER".into(),
            away: "BRA".into(),
        });
        repo.seed_closing_soon(ClosingSoonMatch {
            match_id: 2,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            kickoff_time: Utc::now() + chrono::Duration::hours(5),
            home: "ARG".into(),
            away: "URU".into(),
        });
        // Pretend match 1 has already been notified.
        let _ = repo
            .try_send(&OkNotifier, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();

        let pending = repo.list_closing_soon_unnotified().await.unwrap();
        let ids: Vec<i32> = pending.iter().map(|m| m.match_id).collect();
        assert_eq!(ids, vec![2]);
    }
}
