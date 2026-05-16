//! In-memory [`NotificationRepo`] fake for tests.

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use uuid::Uuid;

use super::{ClosingSoonMatch, NotificationRepo};
use crate::notifier::{NotificationEvent, Notifier};
use crate::repo::RepoResult;

#[derive(Default)]
struct MemoryNotificationState {
    /// (league_id, kind, ref_id, user_id) → marker; mirrors the Pg PK.
    sent: HashSet<(Uuid, String, i32, Option<Uuid>)>,
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
                .insert((league_id, "match_closing_soon".into(), id, None));
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
                    .contains(&(league_id, "match_closing_soon".into(), m.match_id, None))
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
        user_id: Option<Uuid>,
    ) -> RepoResult<bool> {
        Ok(self.state.lock().unwrap().sent.contains(&(
            league_id,
            kind.to_string(),
            ref_id,
            user_id,
        )))
    }

    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
        user_id: Option<Uuid>,
        event: NotificationEvent,
    ) -> RepoResult<bool> {
        // Two-phase emulation of the Pg tx semantics: tentatively reserve the
        // slot, run the notifier, commit on success and release on failure.
        {
            let mut s = self.state.lock().unwrap();
            if !s
                .sent
                .insert((league_id, kind.to_string(), ref_id, user_id))
            {
                return Ok(false);
            }
        }

        match notifier.notify(event).await {
            Ok(()) => Ok(true),
            Err(_) => {
                // Roll back the tentative reservation so the next tick retries.
                self.state.lock().unwrap().sent.remove(&(
                    league_id,
                    kind.to_string(),
                    ref_id,
                    user_id,
                ));
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
    use crate::stage::Stage;
    use chrono::Utc;

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
            .try_send(
                &OkNotifier,
                DEFAULT_LEAGUE_ID,
                "match_closing_soon",
                1,
                None,
                lock_event(),
            )
            .await
            .unwrap();
        assert!(sent);
        assert!(repo
            .already_sent(DEFAULT_LEAGUE_ID, "match_closing_soon", 1, None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn try_send_skips_already_recorded_slot() {
        let repo = MemoryNotificationRepo::new();
        let _ = repo
            .try_send(
                &OkNotifier,
                DEFAULT_LEAGUE_ID,
                "match_closing_soon",
                1,
                None,
                lock_event(),
            )
            .await
            .unwrap();
        let sent_again = repo
            .try_send(
                &OkNotifier,
                DEFAULT_LEAGUE_ID,
                "match_closing_soon",
                1,
                None,
                lock_event(),
            )
            .await
            .unwrap();
        assert!(!sent_again, "second call must be a no-op");
        assert_eq!(repo.sent_count(), 1);
    }

    #[tokio::test]
    async fn try_send_releases_slot_on_notifier_failure() {
        let repo = MemoryNotificationRepo::new();
        let sent = repo
            .try_send(
                &FailNotifier,
                DEFAULT_LEAGUE_ID,
                "special_lock_soon",
                0,
                None,
                lock_event(),
            )
            .await
            .unwrap();
        assert!(!sent);
        assert!(
            !repo
                .already_sent(DEFAULT_LEAGUE_ID, "special_lock_soon", 0, None)
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
        repo.silence_existing_matches(DEFAULT_LEAGUE_ID)
            .await
            .unwrap();

        assert!(repo
            .already_sent(DEFAULT_LEAGUE_ID, "match_closing_soon", 1, None)
            .await
            .unwrap());
        assert!(!repo
            .already_sent(other_league, "match_closing_soon", 1, None)
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
        let missing = repo
            .users_missing_champion(DEFAULT_LEAGUE_ID)
            .await
            .unwrap();
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
            .try_send(
                &OkNotifier,
                DEFAULT_LEAGUE_ID,
                "match_closing_soon",
                1,
                None,
                lock_event(),
            )
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
