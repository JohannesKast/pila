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
    pub phone_number: Option<String>,
    pub jersey_preset: String,
}

/// Full record needed by admin operations (token + phone).
#[derive(Debug, Clone)]
pub struct UserFull {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub phone_number: Option<String>,
    pub is_admin: bool,
}

/// Trimmed projection for admin listings.
#[derive(Debug, Clone)]
pub struct AdminUserRow {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub phone_number: Option<String>,
    pub is_admin: bool,
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
}

#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn find_by_token(&self, token: &str) -> RepoResult<Option<UserAuth>>;
    async fn find_full_by_id(&self, id: Uuid) -> RepoResult<Option<UserFull>>;
    async fn count(&self) -> RepoResult<i64>;
    async fn count_admins(&self) -> RepoResult<i64>;
    async fn list_for_admin(&self) -> RepoResult<Vec<AdminUserRow>>;
    async fn list_basic(&self) -> RepoResult<Vec<UserBasic>>;
    async fn list_ids(&self) -> RepoResult<Vec<Uuid>>;
    async fn create(&self, new_user: NewUser<'_>) -> RepoResult<()>;
    async fn delete(&self, id: Uuid) -> RepoResult<()>;
    async fn set_admin(&self, id: Uuid, is_admin: bool) -> RepoResult<()>;
    async fn rename(&self, id: Uuid, name: &str) -> RepoResult<()>;
    async fn set_jersey(&self, id: Uuid, preset: &str) -> RepoResult<()>;
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
            "SELECT id, name, is_admin, phone_number, jersey_preset FROM users WHERE token = $1",
            token
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| UserAuth {
            id: r.id,
            name: r.name,
            is_admin: r.is_admin,
            phone_number: r.phone_number,
            jersey_preset: r.jersey_preset,
        }))
    }

    async fn find_full_by_id(&self, id: Uuid) -> RepoResult<Option<UserFull>> {
        let row = sqlx::query!(
            "SELECT id, name, token, phone_number, is_admin FROM users WHERE id = $1",
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
            is_admin: r.is_admin,
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

    async fn list_for_admin(&self) -> RepoResult<Vec<AdminUserRow>> {
        let rows = sqlx::query!(
            "SELECT id, name, token, phone_number, is_admin FROM users ORDER BY name"
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
                is_admin: r.is_admin,
            })
            .collect())
    }

    async fn list_basic(&self) -> RepoResult<Vec<UserBasic>> {
        let rows = sqlx::query!("SELECT id, name, jersey_preset FROM users")
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

    async fn list_ids(&self) -> RepoResult<Vec<Uuid>> {
        let ids = sqlx::query_scalar!("SELECT id FROM users")
            .fetch_all(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(ids)
    }

    async fn create(&self, new_user: NewUser<'_>) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO users (id, name, token, is_admin, phone_number) VALUES ($1, $2, $3, $4, $5)",
            new_user.id,
            new_user.name,
            new_user.token,
            new_user.is_admin,
            new_user.phone_number
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
            phone_number: u.phone_number.clone(),
            jersey_preset: s
                .jerseys
                .get(&u.id)
                .cloned()
                .unwrap_or_else(|| "classic".to_string()),
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

    async fn list_for_admin(&self) -> RepoResult<Vec<AdminUserRow>> {
        let s = self.inner.lock().unwrap();
        let mut rows: Vec<AdminUserRow> = s
            .users
            .iter()
            .map(|u| AdminUserRow {
                id: u.id,
                name: u.name.clone(),
                token: u.token.clone(),
                phone_number: u.phone_number.clone(),
                is_admin: u.is_admin,
            })
            .collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows)
    }

    async fn list_basic(&self) -> RepoResult<Vec<UserBasic>> {
        let s = self.inner.lock().unwrap();
        Ok(s.users
            .iter()
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

    async fn list_ids(&self) -> RepoResult<Vec<Uuid>> {
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
            is_admin: new_user.is_admin,
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
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    fn user(name: &str, token: &str, is_admin: bool) -> UserFull {
        UserFull {
            id: Uuid::new_v4(),
            name: name.to_string(),
            token: token.to_string(),
            phone_number: None,
            is_admin,
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
        })
        .await
        .unwrap();

        let auth = repo.find_by_token("tkn-a").await.unwrap().unwrap();
        assert_eq!(auth.id, id);
        assert_eq!(auth.name, "Alice");
        assert!(auth.is_admin);
        assert_eq!(auth.phone_number.as_deref(), Some("+491"));
    }

    #[tokio::test]
    async fn list_for_admin_is_sorted_by_name() {
        let repo = MemoryUserRepo::new();
        repo.seed(user("Charlie", "t1", false), "classic");
        repo.seed(user("Alice", "t2", true), "classic");
        repo.seed(user("Bob", "t3", false), "classic");

        let list = repo.list_for_admin().await.unwrap();
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
}
