// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! In-memory [`UserRepo`] fake for tests.

use async_trait::async_trait;
use std::sync::Mutex;
use uuid::Uuid;

use super::{AdminUserRow, NewUser, UserAuth, UserBasic, UserFull, UserRepo};
use crate::repo::{RepoError, RepoResult};

#[derive(Default)]
struct MemoryUserState {
    users: Vec<UserFull>,
    jerseys: std::collections::HashMap<Uuid, String>,
    /// Colour theme per user; absence means the default `"dark"`. Kept out of
    /// `UserFull` (like `jerseys`) so test fixtures need not construct it.
    themes: std::collections::HashMap<Uuid, String>,
}

/// Lock-protected in-memory implementation for tests.
#[derive(Default)]
pub struct MemoryUserRepo {
    inner: Mutex<MemoryUserState>,
}

impl MemoryUserRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper — seed one user without going through `create` (lets tests
    /// pre-load admins, custom tokens, etc.).
    pub fn seed(&self, user: UserFull, jersey: &str) {
        let mut s = self.inner.lock().unwrap();
        s.jerseys.insert(user.id, jersey.to_string());
        s.users.push(user);
    }
}

#[async_trait]
impl UserRepo for MemoryUserRepo {
    async fn find_by_token(&self, token: &str) -> RepoResult<Option<UserAuth>> {
        let s = self.inner.lock().unwrap();
        Ok(s.users.iter().find(|u| u.token == token).map(|u| UserAuth {
            id: u.id,
            name: u.name.clone(),
            real_name: u.real_name.clone(),
            is_admin: u.is_admin,
            can_create_league: u.can_create_league,
            phone_number: u.phone_number.clone(),
            email: u.email.clone(),
            jersey_preset: s
                .jerseys
                .get(&u.id)
                .cloned()
                .unwrap_or_else(|| "classic".to_string()),
            language: u.language.clone(),
            theme: s
                .themes
                .get(&u.id)
                .cloned()
                .unwrap_or_else(|| "dark".to_string()),
            league_id: u.league_id,
        }))
    }

    async fn find_full_by_id(&self, id: Uuid) -> RepoResult<Option<UserFull>> {
        let s = self.inner.lock().unwrap();
        Ok(s.users.iter().find(|u| u.id == id).cloned())
    }

    async fn count(&self) -> RepoResult<i64> {
        Ok(self.inner.lock().unwrap().users.len() as i64)
    }

    async fn count_admins(&self) -> RepoResult<i64> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .users
            .iter()
            .filter(|u| u.is_admin)
            .count() as i64)
    }

    async fn list_for_admin(&self, league_id: Uuid) -> RepoResult<Vec<AdminUserRow>> {
        let s = self.inner.lock().unwrap();
        let mut rows: Vec<AdminUserRow> = s
            .users
            .iter()
            .filter(|u| u.league_id == league_id)
            .map(|u| AdminUserRow {
                id: u.id,
                name: u.name.clone(),
                real_name: u.real_name.clone(),
                token: u.token.clone(),
                phone_number: u.phone_number.clone(),
                email: u.email.clone(),
                is_admin: u.is_admin,
                can_create_league: u.can_create_league,
            })
            .collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows)
    }

    async fn list_basic(&self, league_id: Uuid) -> RepoResult<Vec<UserBasic>> {
        let s = self.inner.lock().unwrap();
        Ok(s.users
            .iter()
            .filter(|u| u.league_id == league_id)
            .map(|u| UserBasic {
                id: u.id,
                name: u.name.clone(),
                jersey_preset: s
                    .jerseys
                    .get(&u.id)
                    .cloned()
                    .unwrap_or_else(|| "classic".to_string()),
            })
            .collect())
    }

    async fn list_ids(&self, league_id: Uuid) -> RepoResult<Vec<Uuid>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .users
            .iter()
            .filter(|u| u.league_id == league_id)
            .map(|u| u.id)
            .collect())
    }

    async fn list_all_ids(&self) -> RepoResult<Vec<Uuid>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .users
            .iter()
            .map(|u| u.id)
            .collect())
    }

    async fn name_exists(&self, league_id: Uuid, name: &str) -> RepoResult<bool> {
        let s = self.inner.lock().unwrap();
        Ok(s.users
            .iter()
            .any(|u| u.league_id == league_id && u.name.eq_ignore_ascii_case(name)))
    }

    async fn create(&self, new_user: NewUser<'_>) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        // Mirror the DB's unique index on (league_id, lower(name)) so the
        // in-memory repo rejects duplicate names the same way Postgres does.
        if s.users.iter().any(|u| {
            u.league_id == new_user.league_id && u.name.eq_ignore_ascii_case(new_user.name)
        }) {
            return Err(RepoError::Conflict);
        }
        s.jerseys.insert(new_user.id, "classic".to_string());
        s.users.push(UserFull {
            id: new_user.id,
            name: new_user.name.to_string(),
            real_name: new_user.real_name.to_string(),
            token: new_user.token.to_string(),
            phone_number: new_user.phone_number.map(|p| p.to_string()),
            email: new_user.email.map(|e| e.to_string()),
            is_admin: new_user.is_admin,
            can_create_league: false,
            league_id: new_user.league_id,
            language: new_user.language.to_string(),
        });
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        s.users.retain(|u| u.id != id);
        s.jerseys.remove(&id);
        s.themes.remove(&id);
        Ok(())
    }

    async fn set_admin(&self, id: Uuid, is_admin: bool) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        if let Some(u) = s.users.iter_mut().find(|u| u.id == id) {
            u.is_admin = is_admin;
        }
        Ok(())
    }

    async fn set_can_create_league(&self, id: Uuid, can: bool) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        if let Some(u) = s.users.iter_mut().find(|u| u.id == id) {
            u.can_create_league = can;
        }
        Ok(())
    }

    async fn rename(&self, id: Uuid, name: &str) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        // Mirror the DB unique index on (league_id, lower(name)): reject a
        // rename that collides with another user in the same league.
        let target_league = s.users.iter().find(|u| u.id == id).map(|u| u.league_id);
        if let Some(league_id) = target_league {
            if s.users.iter().any(|u| {
                u.id != id && u.league_id == league_id && u.name.eq_ignore_ascii_case(name)
            }) {
                return Err(RepoError::Conflict);
            }
        }
        if let Some(u) = s.users.iter_mut().find(|u| u.id == id) {
            u.name = name.to_string();
        }
        Ok(())
    }

    async fn set_real_name(&self, id: Uuid, real_name: &str) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        if let Some(u) = s.users.iter_mut().find(|u| u.id == id) {
            u.real_name = real_name.to_string();
        }
        Ok(())
    }

    async fn set_jersey(&self, id: Uuid, preset: &str) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        s.jerseys.insert(id, preset.to_string());
        Ok(())
    }

    async fn set_language(&self, id: Uuid, language: &str) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        if let Some(u) = s.users.iter_mut().find(|u| u.id == id) {
            u.language = language.to_string();
        }
        Ok(())
    }

    async fn set_theme(&self, id: Uuid, theme: &str) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        s.themes.insert(id, theme.to_string());
        Ok(())
    }

    async fn set_email(&self, id: Uuid, email: Option<&str>) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        if let Some(u) = s.users.iter_mut().find(|u| u.id == id) {
            u.email = email.map(|e| e.to_string());
        }
        Ok(())
    }

    async fn users_missing_prediction_with_email(
        &self,
        league_id: Uuid,
        _match_id: i32,
    ) -> RepoResult<Vec<(Uuid, String, String, String)>> {
        // Simplified in-memory: returns users with email in the league.
        // Real filtering by match_id is done via the prediction repo in tests.
        let s = self.inner.lock().unwrap();
        Ok(s.users
            .iter()
            .filter(|u| u.league_id == league_id && u.email.is_some())
            .map(|u| {
                (
                    u.id,
                    u.name.clone(),
                    u.email.clone().unwrap(),
                    u.token.clone(),
                )
            })
            .collect())
    }

    async fn users_missing_champion_with_email(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<(Uuid, String, String, String)>> {
        // Simplified: returns all users with email — tests filter further.
        let s = self.inner.lock().unwrap();
        Ok(s.users
            .iter()
            .filter(|u| u.league_id == league_id && u.email.is_some())
            .map(|u| {
                (
                    u.id,
                    u.name.clone(),
                    u.email.clone().unwrap(),
                    u.token.clone(),
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::repo::DEFAULT_LEAGUE_ID;

    fn user(name: &str, token: &str, is_admin: bool) -> UserFull {
        UserFull {
            id: Uuid::new_v4(),
            name: name.to_string(),
            real_name: name.to_string(),
            token: token.to_string(),
            phone_number: None,
            email: None,
            is_admin,
            can_create_league: false,
            league_id: DEFAULT_LEAGUE_ID,
            language: "de".to_string(),
        }
    }

    #[tokio::test]
    async fn create_then_find_by_token_returns_auth_view() {
        let repo = MemoryUserRepo::new();
        let id = Uuid::new_v4();
        repo.create(NewUser {
            id,
            name: "Alice",
            real_name: "Alice",
            token: "tkn-a",
            is_admin: true,
            phone_number: Some("+491"),
            email: Some("alice@example.com"),
            league_id: DEFAULT_LEAGUE_ID,
            language: "de",
        })
        .await
        .unwrap();

        let auth = repo.find_by_token("tkn-a").await.unwrap().unwrap();
        assert_eq!(auth.id, id);
        assert_eq!(auth.name, "Alice");
        assert!(auth.is_admin);
        assert_eq!(auth.phone_number.as_deref(), Some("+491"));
        assert_eq!(auth.email.as_deref(), Some("alice@example.com"));
        assert_eq!(auth.league_id, DEFAULT_LEAGUE_ID);
    }

    #[tokio::test]
    async fn list_for_admin_is_sorted_by_name() {
        let repo = MemoryUserRepo::new();
        repo.seed(user("Charlie", "t1", false), "classic");
        repo.seed(user("Alice", "t2", true), "classic");
        repo.seed(user("Bob", "t3", false), "classic");

        let list = repo.list_for_admin(DEFAULT_LEAGUE_ID).await.unwrap();
        let names: Vec<_> = list.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, vec!["Alice", "Bob", "Charlie"]);
    }

    #[tokio::test]
    async fn count_admins_only_counts_admins() {
        let repo = MemoryUserRepo::new();
        repo.seed(user("a", "t1", true), "classic");
        repo.seed(user("b", "t2", false), "classic");
        repo.seed(user("c", "t3", true), "classic");
        assert_eq!(repo.count_admins().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn rename_updates_name() {
        let repo = MemoryUserRepo::new();
        let u = user("Old", "t", false);
        let id = u.id;
        repo.seed(u, "classic");
        repo.rename(id, "New").await.unwrap();
        let after = repo.find_full_by_id(id).await.unwrap().unwrap();
        assert_eq!(after.name, "New");
    }

    #[tokio::test]
    async fn rename_to_existing_name_in_league_conflicts() {
        let repo = MemoryUserRepo::new();
        repo.seed(user("Alice", "t1", false), "classic");
        let bob = user("Bob", "t2", false);
        let bob_id = bob.id;
        repo.seed(bob, "classic");
        // Case-insensitive collision with another user's tip name is rejected.
        let err = repo.rename(bob_id, "alice").await.unwrap_err();
        assert!(matches!(err, RepoError::Conflict));
        // Renaming to the user's own current name is a no-op, not a conflict.
        repo.rename(bob_id, "Bob").await.unwrap();
    }

    #[tokio::test]
    async fn set_real_name_persists_without_touching_tip_name() {
        let repo = MemoryUserRepo::new();
        let u = user("tippname", "t", false);
        let id = u.id;
        repo.seed(u, "classic");
        repo.set_real_name(id, "Maximilian").await.unwrap();
        let after = repo.find_full_by_id(id).await.unwrap().unwrap();
        assert_eq!(after.real_name, "Maximilian");
        assert_eq!(after.name, "tippname");
    }

    #[tokio::test]
    async fn delete_removes_user_and_jersey() {
        let repo = MemoryUserRepo::new();
        let u = user("X", "t", false);
        let id = u.id;
        repo.seed(u, "brasilien");
        repo.delete(id).await.unwrap();
        assert!(repo.find_full_by_id(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_jersey_persists() {
        let repo = MemoryUserRepo::new();
        let u = user("X", "t", false);
        let id = u.id;
        repo.seed(u, "classic");
        repo.set_jersey(id, "brasilien").await.unwrap();
        let auth = repo.find_by_token("t").await.unwrap().unwrap();
        assert_eq!(auth.jersey_preset, "brasilien");
    }

    #[tokio::test]
    async fn set_theme_defaults_to_dark_then_persists() {
        let repo = MemoryUserRepo::new();
        let u = user("X", "t", false);
        let id = u.id;
        repo.seed(u, "classic");
        // Unseen user defaults to dark.
        assert_eq!(
            repo.find_by_token("t").await.unwrap().unwrap().theme,
            "dark"
        );
        repo.set_theme(id, "light").await.unwrap();
        assert_eq!(
            repo.find_by_token("t").await.unwrap().unwrap().theme,
            "light"
        );
    }

    #[tokio::test]
    async fn set_email_persists() {
        let repo = MemoryUserRepo::new();
        let u = user("X", "t", false);
        let id = u.id;
        repo.seed(u, "classic");
        repo.set_email(id, Some("x@example.com")).await.unwrap();
        let full = repo.find_full_by_id(id).await.unwrap().unwrap();
        assert_eq!(full.email.as_deref(), Some("x@example.com"));
        repo.set_email(id, None).await.unwrap();
        let full = repo.find_full_by_id(id).await.unwrap().unwrap();
        assert!(full.email.is_none());
    }

    #[tokio::test]
    async fn list_for_admin_filters_by_league() {
        let repo = MemoryUserRepo::new();
        let other_league = Uuid::new_v4();
        let mut a = user("Alice", "t1", false);
        a.league_id = DEFAULT_LEAGUE_ID;
        let mut b = user("Bob", "t2", false);
        b.league_id = other_league;
        repo.seed(a, "classic");
        repo.seed(b, "classic");

        let default_list = repo.list_for_admin(DEFAULT_LEAGUE_ID).await.unwrap();
        assert_eq!(default_list.len(), 1);
        assert_eq!(default_list[0].name, "Alice");

        let other_list = repo.list_for_admin(other_league).await.unwrap();
        assert_eq!(other_list.len(), 1);
        assert_eq!(other_list[0].name, "Bob");
    }

    #[tokio::test]
    async fn list_ids_filters_by_league() {
        let repo = MemoryUserRepo::new();
        let other = Uuid::new_v4();
        let mut a = user("Alice", "t1", false);
        a.league_id = DEFAULT_LEAGUE_ID;
        let mut b = user("Bob", "t2", false);
        b.league_id = other;
        repo.seed(a, "classic");
        repo.seed(b, "classic");

        assert_eq!(repo.list_ids(DEFAULT_LEAGUE_ID).await.unwrap().len(), 1);
        assert_eq!(repo.list_ids(other).await.unwrap().len(), 1);
        assert_eq!(repo.list_all_ids().await.unwrap().len(), 2);
    }
}
