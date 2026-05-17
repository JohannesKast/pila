// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

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
use uuid::Uuid;

use super::RepoResult;
use crate::notifier::{NotificationEvent, Notifier};
use crate::stage::Stage;

mod memory;
mod postgres;

pub use memory::MemoryNotificationRepo;
pub use postgres::PgNotificationRepo;

/// Sentinel UUID used for group notifications where no specific user
/// is targeted (Signal group messages). PK columns cannot be NULL.
pub(super) const NO_USER: Uuid = Uuid::from_u128(0);

/// Convert an optional user_id to the sentinel for DB storage.
pub(super) fn user_id_for_db(user_id: Option<Uuid>) -> Uuid {
    user_id.unwrap_or(NO_USER)
}

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

    /// Whether `(league_id, kind, ref_id, user_id)` has already been recorded as sent.
    async fn already_sent(
        &self,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
        user_id: Option<Uuid>,
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
    ///
    /// `user_id` is `None` for group notifications (Signal), `Some(id)`
    /// for per-user notifications (email).
    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
        user_id: Option<Uuid>,
        event: NotificationEvent,
    ) -> RepoResult<bool>;
}
