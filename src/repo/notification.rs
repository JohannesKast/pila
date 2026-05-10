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
use uuid::Uuid;

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
    /// Insert a sentinel `match_closing_soon` row for every currently-known
    /// fixture so the very first worker tick after a fresh deploy does not
    /// flood the group with retroactive reminders. Scoped per-league so each
    /// league bootstraps independently.
    async fn silence_existing_matches(&self, league_id: Uuid) -> RepoResult<()>;

    /// Matches with a kickoff in the next 24h that have not yet had a
    /// `match_closing_soon` row recorded for `league_id`.
    async fn list_closing_soon_unnotified(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<ClosingSoonMatch>>;

    /// Names of users in `league_id` without a tip on the given match.
    async fn users_missing_prediction_for(
        &self,
        league_id: Uuid,
        match_id: i32,
    ) -> RepoResult<Vec<String>>;

    /// Names of users in `league_id` without a champion pick.
    async fn users_missing_champion(&self, league_id: Uuid) -> RepoResult<Vec<String>>;

    /// Whether `(league_id, kind, ref_id)` has already been recorded as sent.
    async fn already_sent(
        &self,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
    ) -> RepoResult<bool>;

    /// Atomic dispatch primitive. In one transaction:
    ///   1. INSERT into `sent_notifications` ON CONFLICT DO NOTHING.
    ///   2. If the insert was a no-op (already recorded), rollback and
    ///      return `false`.
    ///   3. Otherwise call `notifier.notify(event)`. On success commit and
    ///      return `true`. On failure rollback (so the next worker tick
    ///      retries) and return `false`.
    ///
    /// Idempotency is partitioned by `league_id` — two leagues can each
    /// receive the same `(kind, ref_id)` independently.
    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        league_id: Uuid,
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
    async fn silence_existing_matches(&self, league_id: Uuid) -> RepoResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO sent_notifications (league_id, kind, ref_id)
            SELECT $1, 'match_closing_soon', m.id FROM matches m
            WHERE m.team_home_id IS NOT NULL AND m.team_away_id IS NOT NULL
            ON CONFLICT (league_id, kind, ref_id) DO NOTHING
            "#,
            league_id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn list_closing_soon_unnotified(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<ClosingSoonMatch>> {
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
              ON n.kind = 'match_closing_soon' AND n.ref_id = m.id AND n.league_id = $1
            WHERE m.team_home_id IS NOT NULL AND m.team_away_id IS NOT NULL
              AND m.kickoff_time IS NOT NULL
              AND m.kickoff_time BETWEEN NOW() AND NOW() + INTERVAL '24 hours'
              AND n.ref_id IS NULL
            "#,
            league_id
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

    async fn users_missing_prediction_for(
        &self,
        league_id: Uuid,
        match_id: i32,
    ) -> RepoResult<Vec<String>> {
        let names = sqlx::query_scalar!(
            r#"
            SELECT u.name FROM users u
            WHERE u.league_id = $1
              AND NOT EXISTS (
                SELECT 1 FROM predictions p
                WHERE p.user_id = u.id AND p.match_id = $2
            )
            ORDER BY u.name
            "#,
            league_id,
            match_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(names)
    }

    async fn users_missing_champion(&self, league_id: Uuid) -> RepoResult<Vec<String>> {
        let names = sqlx::query_scalar!(
            r#"
            SELECT u.name FROM users u
            WHERE u.league_id = $1
              AND NOT EXISTS (
                SELECT 1 FROM special_predictions sp
                WHERE sp.user_id = u.id AND sp.champion_id IS NOT NULL
            )
            ORDER BY u.name
            "#,
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(names)
    }

    async fn already_sent(
        &self,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
    ) -> RepoResult<bool> {
        let row = sqlx::query!(
            "SELECT 1 AS dummy FROM sent_notifications \
             WHERE league_id = $1 AND kind = $2 AND ref_id = $3",
            league_id,
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
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
        event: NotificationEvent,
    ) -> RepoResult<bool> {
        let mut tx = self.pool.begin().await.map_err(RepoError::from)?;

        let inserted = sqlx::query_scalar!(
            "INSERT INTO sent_notifications (league_id, kind, ref_id) VALUES ($1, $2, $3)
             ON CONFLICT (league_id, kind, ref_id) DO NOTHING
             RETURNING ref_id",
            league_id,
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
                tracing::info!("Sent notification: league={} {} {}", league_id, kind, ref_id);
                Ok(true)
            }
            Err(e) => {
                tracing::error!(
                    "Notifier failed for league={} {} {}: {:?}",
                    league_id,
                    kind,
                    ref_id,
                    e
                );
                let _ = tx.rollback().await;
                Ok(false)
            }
        }
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

#[derive(Default)]
struct MemoryNotificationState {
    /// (league_id, kind, ref_id) → marker; mirrors the Pg PK.
    sent: HashSet<(Uuid, String, i32)>,
    /// Matches indexed by id. Test code seeds this so the queries have rows
    /// to find. Keeps the fake decoupled from `MemoryMatchRepo`.
    closing_soon: Vec<ClosingSoonMatch>,
    /// (league_id, user_name) → set of match_ids they have tipped.
    predictions: HashMap<(Uuid, String), HashSet<i32>>,
    /// (league_id, user_name) — all registered users.
    user_names: Vec<(Uuid, String)>,
    /// (league_id, user_name) — users with a champion pick.
    champion_picked: HashSet<(Uuid, String)>,
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

    pub fn seed_user(&self, league_id: Uuid, name: &str) {
        let mut s = self.state.lock().unwrap();
        s.user_names.push((league_id, name.to_string()));
    }

    pub fn seed_user_with_champion(&self, league_id: Uuid, name: &str) {
        let mut s = self.state.lock().unwrap();
        s.user_names.push((league_id, name.to_string()));
        s.champion_picked.insert((league_id, name.to_string()));
    }

    pub fn seed_prediction(&self, league_id: Uuid, user_name: &str, match_id: i32) {
        let mut s = self.state.lock().unwrap();
        s.predictions
            .entry((league_id, user_name.to_string()))
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

    pub fn sent_count(&self) -> usize {
        self.state.lock().unwrap().sent.len()
    }
}

#[async_trait]
impl NotificationRepo for MemoryNotificationRepo {
    async fn silence_existing_matches(&self, league_id: Uuid) -> RepoResult<()> {
        let mut s = self.state.lock().unwrap();
        let ids: Vec<i32> = s.matches_with_both_teams.clone();
        for id in ids {
            s.sent
                .insert((league_id, "match_closing_soon".into(), id));
        }
        Ok(())
    }

    async fn list_closing_soon_unnotified(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<ClosingSoonMatch>> {
        let s = self.state.lock().unwrap();
        Ok(s.closing_soon
            .iter()
            .filter(|m| {
                !s.sent
                    .contains(&(league_id, "match_closing_soon".into(), m.match_id))
            })
            .cloned()
            .collect())
    }

    async fn users_missing_prediction_for(
        &self,
        league_id: Uuid,
        match_id: i32,
    ) -> RepoResult<Vec<String>> {
        let s = self.state.lock().unwrap();
        let mut names: Vec<String> = s
            .user_names
            .iter()
            .filter(|(lid, _)| *lid == league_id)
            .filter(|(lid, n)| {
                !s.predictions
                    .get(&(*lid, n.clone()))
                    .is_some_and(|m| m.contains(&match_id))
            })
            .map(|(_, n)| n.clone())
            .collect();
        names.sort();
        Ok(names)
    }

    async fn users_missing_champion(&self, league_id: Uuid) -> RepoResult<Vec<String>> {
        let s = self.state.lock().unwrap();
        let mut names: Vec<String> = s
            .user_names
            .iter()
            .filter(|(lid, _)| *lid == league_id)
            .filter(|(lid, n)| !s.champion_picked.contains(&(*lid, n.clone())))
            .map(|(_, n)| n.clone())
            .collect();
        names.sort();
        Ok(names)
    }

    async fn already_sent(
        &self,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
    ) -> RepoResult<bool> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .sent
            .contains(&(league_id, kind.to_string(), ref_id)))
    }

    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
        event: NotificationEvent,
    ) -> RepoResult<bool> {
        // Two-phase emulation of the Pg tx semantics: tentatively reserve the
        // slot, run the notifier, commit on success and release on failure.
        {
            let mut s = self.state.lock().unwrap();
            if !s.sent.insert((league_id, kind.to_string(), ref_id)) {
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
                    .remove(&(league_id, kind.to_string(), ref_id));
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::notifier::NotifierError;
    use crate::repo::DEFAULT_LEAGUE_ID;

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
            .try_send(&OkNotifier, DEFAULT_LEAGUE_ID, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();
        assert!(sent);
        assert!(repo
            .already_sent(DEFAULT_LEAGUE_ID, "match_closing_soon", 1)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn try_send_skips_already_recorded_slot() {
        let repo = MemoryNotificationRepo::new();
        let _ = repo
            .try_send(&OkNotifier, DEFAULT_LEAGUE_ID, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();
        let sent_again = repo
            .try_send(&OkNotifier, DEFAULT_LEAGUE_ID, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();
        assert!(!sent_again, "second call must be a no-op");
        assert_eq!(repo.sent_count(), 1);
    }

    #[tokio::test]
    async fn try_send_releases_slot_on_notifier_failure() {
        let repo = MemoryNotificationRepo::new();
        let sent = repo
            .try_send(&FailNotifier, DEFAULT_LEAGUE_ID, "special_lock_soon", 0, lock_event())
            .await
            .unwrap();
        assert!(!sent);
        assert!(
            !repo
                .already_sent(DEFAULT_LEAGUE_ID, "special_lock_soon", 0)
                .await
                .unwrap(),
            "slot must be released so the next tick retries"
        );
    }

    #[tokio::test]
    async fn silence_only_marks_within_one_league() {
        let repo = MemoryNotificationRepo::new();
        let other_league = Uuid::new_v4();
        repo.seed_match_with_both_teams(1);
        repo.seed_match_with_both_teams(2);
        repo.silence_existing_matches(DEFAULT_LEAGUE_ID).await.unwrap();

        assert!(repo
            .already_sent(DEFAULT_LEAGUE_ID, "match_closing_soon", 1)
            .await
            .unwrap());
        assert!(!repo
            .already_sent(other_league, "match_closing_soon", 1)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn users_missing_prediction_excludes_those_who_tipped() {
        let repo = MemoryNotificationRepo::new();
        repo.seed_user(DEFAULT_LEAGUE_ID, "Anna");
        repo.seed_user(DEFAULT_LEAGUE_ID, "Ben");
        repo.seed_user(DEFAULT_LEAGUE_ID, "Cleo");
        repo.seed_prediction(DEFAULT_LEAGUE_ID, "Ben", 5);

        let missing = repo
            .users_missing_prediction_for(DEFAULT_LEAGUE_ID, 5)
            .await
            .unwrap();
        assert_eq!(missing, vec!["Anna".to_string(), "Cleo".to_string()]);
    }

    #[tokio::test]
    async fn users_missing_prediction_excludes_other_leagues() {
        let repo = MemoryNotificationRepo::new();
        let other_league = Uuid::new_v4();
        repo.seed_user(DEFAULT_LEAGUE_ID, "Anna");
        repo.seed_user(other_league, "Mallory");
        let missing = repo
            .users_missing_prediction_for(DEFAULT_LEAGUE_ID, 5)
            .await
            .unwrap();
        assert_eq!(missing, vec!["Anna".to_string()]);
    }

    #[tokio::test]
    async fn users_missing_champion_excludes_pickers() {
        let repo = MemoryNotificationRepo::new();
        repo.seed_user(DEFAULT_LEAGUE_ID, "Anna");
        repo.seed_user_with_champion(DEFAULT_LEAGUE_ID, "Ben");
        let missing = repo.users_missing_champion(DEFAULT_LEAGUE_ID).await.unwrap();
        assert_eq!(missing, vec!["Anna".to_string()]);
    }

    #[tokio::test]
    async fn list_closing_soon_excludes_already_sent_for_this_league_only() {
        let repo = MemoryNotificationRepo::new();
        let other_league = Uuid::new_v4();
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
        // Default league marked match 1 as sent.
        let _ = repo
            .try_send(&OkNotifier, DEFAULT_LEAGUE_ID, "match_closing_soon", 1, lock_event())
            .await
            .unwrap();

        // Default league sees only match 2 as pending.
        let default_pending = repo
            .list_closing_soon_unnotified(DEFAULT_LEAGUE_ID)
            .await
            .unwrap();
        let default_ids: Vec<i32> = default_pending.iter().map(|m| m.match_id).collect();
        assert_eq!(default_ids, vec![2]);

        // Other league still sees both — its idempotency state is independent.
        let other_pending = repo
            .list_closing_soon_unnotified(other_league)
            .await
            .unwrap();
        let other_ids: Vec<i32> = other_pending.iter().map(|m| m.match_id).collect();
        assert_eq!(other_ids, vec![1, 2]);
    }
}
