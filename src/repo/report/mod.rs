// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! AI matchday-recap persistence.
//!
//! One recap per `(league_id, matchday_date)`. The background worker writes
//! exactly one row per finished matchday; the dashboard reads the latest (or a
//! navigated-to) recap. Like every other aggregate, recaps are league-scoped.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use super::RepoResult;

mod memory;
mod postgres;

pub use memory::MemoryMatchdayReportRepo;
pub use postgres::PgMatchdayReportRepo;

/// A stored matchday recap.
#[derive(Debug, Clone)]
pub struct MatchdayReport {
    pub league_id: Uuid,
    pub matchday_date: NaiveDate,
    pub language: String,
    /// Markdown body as returned by the model.
    pub content: String,
    pub model: String,
    pub generated_at: DateTime<Utc>,
}

#[async_trait]
pub trait MatchdayReportRepo: Send + Sync {
    /// The recap for one matchday, or `None` if it has not been generated.
    async fn get(&self, league_id: Uuid, date: NaiveDate) -> RepoResult<Option<MatchdayReport>>;

    /// Whether a recap already exists for `(league_id, date)`. Lets the worker
    /// skip already-generated matchdays cheaply.
    async fn exists(&self, league_id: Uuid, date: NaiveDate) -> RepoResult<bool>;

    /// The most recent matchday that has a recap, or `None` if the league has
    /// none yet. Drives the default view on the dashboard.
    async fn latest_date(&self, league_id: Uuid) -> RepoResult<Option<NaiveDate>>;

    /// Neighbouring matchday dates with recaps for arrow navigation:
    /// `(older, newer)` relative to `date`. Either side is `None` at the ends.
    async fn neighbors(
        &self,
        league_id: Uuid,
        date: NaiveDate,
    ) -> RepoResult<(Option<NaiveDate>, Option<NaiveDate>)>;

    /// Insert a recap. Idempotent: a second insert for the same
    /// `(league_id, matchday_date)` is a no-op so a racing worker tick cannot
    /// create duplicates.
    async fn insert(&self, report: &MatchdayReport) -> RepoResult<()>;
}
