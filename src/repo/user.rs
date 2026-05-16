//! User persistence.

use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;

use super::{RepoError, RepoResult};

/// Authenticated-user view. Mirrors what the auth extractor needs.
#[derive(Debug, Clone)]
pub struct UserAuth {
    pub id: Uuid,
    pub name: String,
    pub is_admin: bool,
    pub can_create_league: bool,
    pub phone_number: Option<String>,
    pub email: Option<String>,
    pub jersey_preset: String,
    pub language: String,
    pub league_id: Uuid,
}

/// Full record needed by admin operations (token + phone + email).
#[derive(Debug, Clone)]
pub struct UserFull {
    pub id: Uuid,
    pub name: String,
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
    pub token: &'a str,
    pub is_admin: bool,
    pub phone_number: Option<&'a str>,
    pub email: Option<&'a str>,
    pub league_id: Uuid,
    pub language: &'a str,
}

#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn find_by_token(&self, token: &str) -> RepoResult<Option<UserAuth>>;
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
    async fn create(&self, new_user: NewUser<'_>) -> RepoResult<()>;
    async fn delete(&self, id: Uuid) -> RepoResult<()>;
    async fn set_admin(&self, id: Uuid, is_admin: bool) -> RepoResult<()>;
    async fn set_can_create_league(&self, id: Uuid, can: bool) -> RepoResult<()>;
    async fn rename(&self, id: Uuid, name: &str) -> RepoResult<()>;
    async fn set_jersey(&self, id: Uuid, preset: &str) -> RepoResult<()>;
    async fn set_language(&self, id: Uuid, language: &str) -> RepoResult<()>;
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

// ─── Postgres implementation ─────────────────────────────────────────────────

pub struct PgUserRepo {
    pool: PgPool,
}

impl PgUserRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserRepo for PgUserRepo {
    async fn find_by_token(&self, token: &str) -> RepoResult<Option<UserAuth>> {
        let row = sqlx::query!(
            "SELECT id, name, is_admin, can_create_league, phone_number, email, jersey_preset, language, league_id \
             FROM users WHERE token = $1",
            token
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| UserAuth {
            id: r.id,
            name: r.name,
            is_admin: r.is_admin,
            can_create_league: r.can_create_league,
            phone_number: r.phone_number,
            email: r.email,
            jersey_preset: r.jersey_preset,
            language: r.language,
            league_id: r.league_id,
        }))
    }

    async fn find_full_by_id(&self, id: Uuid) -> RepoResult<Option<UserFull>> {
        let row = sqlx::query!(
            "SELECT id, name, token, phone_number, email, is_admin, can_create_league, league_id, language \
             FROM users WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| UserFull {
            id: r.id,
            name: r.name,
            token: r.token,
            phone_number: r.phone_number,
            email: r.email,
            is_admin: r.is_admin,
            can_create_league: r.can_create_league,
            league_id: r.league_id,
            language: r.language,
        }))
    }

    async fn count(&self) -> RepoResult<i64> {
        let c = sqlx::query_scalar!("SELECT COUNT(*) AS \"c!\" FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(c)
    }

    async fn count_admins(&self) -> RepoResult<i64> {
        let c = sqlx::query_scalar!("SELECT COUNT(*) AS \"c!\" FROM users WHERE is_admin")
            .fetch_one(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(c)
    }

    async fn list_for_admin(&self, league_id: Uuid) -> RepoResult<Vec<AdminUserRow>> {
        let rows = sqlx::query!(
            "SELECT id, name, token, phone_number, email, is_admin, can_create_league \
             FROM users WHERE league_id = $1 ORDER BY name",
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| AdminUserRow {
                id: r.id,
                name: r.name,
                token: r.token,
                phone_number: r.phone_number,
                email: r.email,
                is_admin: r.is_admin,
                can_create_league: r.can_create_league,
            })
            .collect())
    }

    async fn list_basic(&self, league_id: Uuid) -> RepoResult<Vec<UserBasic>> {
        let rows = sqlx::query!(
            "SELECT id, name, jersey_preset FROM users WHERE league_id = $1",
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| UserBasic {
                id: r.id,
                name: r.name,
                jersey_preset: r.jersey_preset,
            })
            .collect())
    }

    async fn list_ids(&self, league_id: Uuid) -> RepoResult<Vec<Uuid>> {
        let ids = sqlx::query_scalar!(
            "SELECT id FROM users WHERE league_id = $1",
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(ids)
    }

    async fn list_all_ids(&self) -> RepoResult<Vec<Uuid>> {
        let ids = sqlx::query_scalar!("SELECT id FROM users")
            .fetch_all(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(ids)
    }

    async fn create(&self, new_user: NewUser<'_>) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO users (id, name, token, is_admin, phone_number, email, league_id, language) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            new_user.id,
            new_user.name,
            new_user.token,
            new_user.is_admin,
            new_user.phone_number,
            new_user.email,
            new_user.league_id,
            new_user.language
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> RepoResult<()> {
        sqlx::query!("DELETE FROM users WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(())
    }

    async fn set_admin(&self, id: Uuid, is_admin: bool) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE users SET is_admin = $1 WHERE id = $2",
            is_admin,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn set_can_create_league(&self, id: Uuid, can: bool) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE users SET can_create_league = $1 WHERE id = $2",
            can,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn rename(&self, id: Uuid, name: &str) -> RepoResult<()> {
        sqlx::query!("UPDATE users SET name = $1 WHERE id = $2", name, id)
            .execute(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(())
    }

    async fn set_jersey(&self, id: Uuid, preset: &str) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE users SET jersey_preset = $1 WHERE id = $2",
            preset,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn set_language(&self, id: Uuid, language: &str) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE users SET language = $1 WHERE id = $2",
            language,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn set_email(&self, id: Uuid, email: Option<&str>) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE users SET email = $1 WHERE id = $2",
            email,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn users_missing_prediction_with_email(
        &self,
        league_id: Uuid,
        match_id: i32,
    ) -> RepoResult<Vec<(Uuid, String, String, String)>> {
        let rows = sqlx::query!(
            r#"
            SELECT u.id, u.name, u.email AS "email!", u.token
            FROM users u
            WHERE u.league_id = $1
              AND u.email IS NOT NULL
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

        Ok(rows
            .into_iter()
            .map(|r| (r.id, r.name, r.email, r.token))
            .collect())
    }

    async fn users_missing_champion_with_email(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<(Uuid, String, String, String)>> {
        let rows = sqlx::query!(
            r#"
            SELECT u.id, u.name, u.email AS "email!", u.token
            FROM users u
            WHERE u.league_id = $1
              AND u.email IS NOT NULL
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

        Ok(rows
            .into_iter()
            .map(|r| (r.id, r.name, r.email, r.token))
            .collect())
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

#[derive(Default)]
struct MemoryUserState {
    users: Vec<UserFull>,
    jerseys: std::collections::HashMap<Uuid, String>,
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
            is_admin: u.is_admin,
            can_create_league: u.can_create_league,
            phone_number: u.phone_number.clone(),
            email: u.email.clone(),
            jersey_preset: s
                .jerseys
                .get(&u.id)
                .cloned()
                .unwrap_or_else(|| "classic".to_string()),
            language: "de".to_string(),
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

    async fn create(&self, new_user: NewUser<'_>) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        s.jerseys.insert(new_user.id, "classic".to_string());
        s.users.push(UserFull {
            id: new_user.id,
            name: new_user.name.to_string(),
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
        if let Some(u) = s.users.iter_mut().find(|u| u.id == id) {
            u.name = name.to_string();
        }
        Ok(())
    }

    async fn set_jersey(&self, id: Uuid, preset: &str) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        s.jerseys.insert(id, preset.to_string());
        Ok(())
    }

    async fn set_language(&self, _id: Uuid, _language: &str) -> RepoResult<()> {
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
            .map(|u| (u.id, u.name.clone(), u.email.clone().unwrap(), u.token.clone()))
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
            .map(|u| (u.id, u.name.clone(), u.email.clone().unwrap(), u.token.clone()))
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
