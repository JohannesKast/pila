// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! User persistence.

use async_trait::async_trait;
use uuid::Uuid;

use super::RepoResult;

mod memory;
mod postgres;

pub use memory::MemoryUserRepo;
pub use postgres::PgUserRepo;

/// Authenticated-user view. Mirrors what the auth extractor needs.
#[derive(Debug, Clone)]
pub struct UserAuth {
    pub id: Uuid,
    pub name: String,
    /// Private real first name. Only ever shown back to the user themselves
    /// (in the profile editor) and to league admins — never to other players.
    pub real_name: String,
    pub is_admin: bool,
    pub can_create_league: bool,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub jersey_preset: String,
    pub language: String,
    /// Colour theme preference: `"dark"` (default) or `"light"`.
    pub theme: String,
    pub league_id: Uuid,
}

/// Full record needed by admin operations (token + phone + email).
#[derive(Debug, Clone)]
pub struct UserFull {
    pub id: Uuid,
    pub name: String,
    /// Private real first name (admin-visible only). See [`UserAuth::real_name`].
    pub real_name: String,
    pub token: String,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub is_admin: bool,
    pub can_create_league: bool,
    pub league_id: Uuid,
    pub language: String,
}

/// Trimmed projection for admin listings.
#[derive(Debug, Clone)]
pub struct AdminUserRow {
    pub id: Uuid,
    pub name: String,
    /// Private real first name — surfaced in the admin user list so an admin
    /// can tell who is behind a playful tip name.
    pub real_name: String,
    pub token: String,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub is_admin: bool,
    pub can_create_league: bool,
}

/// Lightweight tuple for leaderboard / jersey lookups.
#[derive(Debug, Clone)]
pub struct UserBasic {
    pub id: Uuid,
    pub name: String,
    pub jersey_preset: String,
}

/// Input for `create`.
#[derive(Debug, Clone)]
pub struct NewUser<'a> {
    pub id: Uuid,
    pub name: &'a str,
    /// Private real first name. Callers that have no separate value pass the
    /// tip name here, matching the production default of `real_name = name`.
    pub real_name: &'a str,
    pub token: &'a str,
    pub is_admin: bool,
    pub phone_number: Option<&'a str>,
    pub email: Option<&'a str>,
    pub league_id: Uuid,
    pub language: &'a str,
}

#[async_trait]
pub trait UserRepo: Send + Sync {
    /// Looks up the user whose magic-link token matches. Returns `None` if
    /// the token is unknown.
    async fn find_by_token(&self, token: &str) -> RepoResult<Option<UserAuth>>;
    /// Full user record needed by admin views. Returns `None` if the user
    /// does not exist.
    async fn find_full_by_id(&self, id: Uuid) -> RepoResult<Option<UserFull>>;
    /// Global user count — the `/setup` route uses this to detect a fresh
    /// install (no users at all). Stays global on purpose.
    async fn count(&self) -> RepoResult<i64>;
    /// Global admin count — used to refuse demoting the last admin overall.
    async fn count_admins(&self) -> RepoResult<i64>;
    /// Admin user list scoped to a single league.
    async fn list_for_admin(&self, league_id: Uuid) -> RepoResult<Vec<AdminUserRow>>;
    /// Basic user info (name + jersey) for leaderboard rendering, scoped to
    /// one league.
    async fn list_basic(&self, league_id: Uuid) -> RepoResult<Vec<UserBasic>>;
    /// All user ids in a league — denominator for badge calculations.
    async fn list_ids(&self, league_id: Uuid) -> RepoResult<Vec<Uuid>>;
    /// All user ids across every league — used by the worker's per-league
    /// notification dispatch loop.
    async fn list_all_ids(&self) -> RepoResult<Vec<Uuid>>;
    /// Case-insensitive check whether a display name is already taken inside
    /// the league. Used to reject duplicate self-registrations before insert;
    /// the unique index on `(league_id, lower(name))` is the race-safe backstop.
    async fn name_exists(&self, league_id: Uuid, name: &str) -> RepoResult<bool>;
    /// Inserts a new user. Fails with `RepoError::Conflict` if the display
    /// name is already taken in the league, or if the id or token collide.
    async fn create(&self, new_user: NewUser<'_>) -> RepoResult<()>;
    /// Deletes the user and all dependent rows (predictions, special_predictions).
    async fn delete(&self, id: Uuid) -> RepoResult<()>;
    /// Grants or revokes the `is_admin` flag. Use `count_admins` before
    /// revoking to avoid demoting the last admin.
    async fn set_admin(&self, id: Uuid, is_admin: bool) -> RepoResult<()>;
    /// Grants or revokes the super-admin `can_create_league` permission.
    async fn set_can_create_league(&self, id: Uuid, can: bool) -> RepoResult<()>;
    /// Updates the user's public tip name. Fails with `RepoError::Conflict`
    /// if another user in the same league already uses the name (the unique
    /// index on `(league_id, lower(name))` is the race-safe backstop).
    async fn rename(&self, id: Uuid, name: &str) -> RepoResult<()>;
    /// Updates the user's private real name. No uniqueness constraint —
    /// two players may legitimately share a first name.
    async fn set_real_name(&self, id: Uuid, real_name: &str) -> RepoResult<()>;
    /// Persists the user's chosen jersey preset key.
    async fn set_jersey(&self, id: Uuid, preset: &str) -> RepoResult<()>;
    /// Persists the user's preferred locale code.
    async fn set_language(&self, id: Uuid, language: &str) -> RepoResult<()>;
    /// Persists the user's colour theme (`"dark"` or `"light"`).
    async fn set_theme(&self, id: Uuid, theme: &str) -> RepoResult<()>;
    /// Sets or clears the user's email address. Pass `None` to remove.
    async fn set_email(&self, id: Uuid, email: Option<&str>) -> RepoResult<()>;
    /// Users in a league who have an email address and are missing a
    /// prediction for the given match. Returns (user_id, name, email, token).
    async fn users_missing_prediction_with_email(
        &self,
        league_id: Uuid,
        match_id: i32,
    ) -> RepoResult<Vec<(Uuid, String, String, String)>>;
    /// Users in a league who have an email and no champion pick.
    /// Returns (user_id, name, email, token).
    async fn users_missing_champion_with_email(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<(Uuid, String, String, String)>>;
}
