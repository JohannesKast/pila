// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Invite-link persistence.
//!
//! An invite link is a league-scoped, shareable secret. Anyone holding the
//! token can self-register a user in the link's league through the public
//! `/join/{token}` flow. Revoking a link deletes its row so the token is no
//! longer recognised.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::RepoResult;

mod memory;
mod postgres;

pub use memory::MemoryInviteRepo;
pub use postgres::PgInviteRepo;

/// One shareable invite link.
#[derive(Debug, Clone)]
pub struct InviteLink {
    pub id: Uuid,
    /// Tenancy boundary: users who join via this link land in this league.
    pub league_id: Uuid,
    /// Secret carried in the public join URL.
    pub token: String,
    /// Optional admin-facing note distinguishing multiple links.
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait InviteRepo: Send + Sync {
    /// Creates a new invite link for a league and returns its id.
    async fn create(&self, league_id: Uuid, token: &str, label: Option<&str>) -> RepoResult<Uuid>;
    /// All invite links for one league, newest first.
    async fn list_for_league(&self, league_id: Uuid) -> RepoResult<Vec<InviteLink>>;
    /// Looks up a link by its token. Returns `None` for unknown/revoked
    /// tokens — the public join flow uses this to validate the URL and learn
    /// which league the new user belongs to.
    async fn find_by_token(&self, token: &str) -> RepoResult<Option<InviteLink>>;
    /// Looks up a link by id. Used by the revoke route to resolve the link's
    /// league before the per-league access check.
    async fn find_by_id(&self, id: Uuid) -> RepoResult<Option<InviteLink>>;
    /// Revokes (deletes) a link. The token stops working immediately.
    async fn delete(&self, id: Uuid) -> RepoResult<()>;
}
