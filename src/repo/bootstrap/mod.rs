// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Bootstrap repo — creates the first league, its settings, and the super-admin
//! user in a single atomic transaction. Used only by the `/setup` handler.

use async_trait::async_trait;
use uuid::Uuid;

use super::RepoResult;

mod memory;
mod postgres;

pub use memory::MemoryBootstrapRepo;
pub use postgres::PgBootstrapRepo;

/// Parameters for the first-run setup transaction.
pub struct FirstLeagueParams<'a> {
    pub user_id: Uuid,
    pub user_name: &'a str,
    /// Private real first name of the first admin. The `/setup` form lets the
    /// admin enter it; if left blank it defaults to `user_name`.
    pub user_real_name: &'a str,
    pub token: &'a str,
    pub phone_number: Option<&'a str>,
    pub email: Option<&'a str>,
    pub language: &'a str,
    pub league_name: &'a str,
    /// All (key, value) pairs for league_settings; empty values are skipped.
    pub settings: &'a [(&'a str, &'a str)],
}

#[async_trait]
pub trait BootstrapRepo: Send + Sync {
    /// Creates the first league, its settings, and the super-admin user in one
    /// atomic transaction. Rolls back on any failure — no half-created state
    /// can exist after an error.
    async fn create_first_league_and_admin(
        &self,
        params: FirstLeagueParams<'_>,
    ) -> RepoResult<Uuid>;
}
