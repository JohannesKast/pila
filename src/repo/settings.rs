//! Key/value settings persistence.

use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Mutex;

use super::{RepoError, RepoResult};

#[async_trait]
pub trait SettingsRepo: Send + Sync {
    async fn get(&self, key: &str) -> RepoResult<Option<String>>;
    async fn set(&self, key: &str, value: &str) -> RepoResult<()>;
}

// ─── Postgres implementation ─────────────────────────────────────────────────

pub struct PgSettingsRepo {
    pool: PgPool,
}

impl PgSettingsRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SettingsRepo for PgSettingsRepo {
    async fn get(&self, key: &str) -> RepoResult<Option<String>> {
        let row = sqlx::query_scalar!("SELECT value FROM settings WHERE key = $1", key)
            .fetch_optional(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(row.flatten())
    }

    async fn set(&self, key: &str, value: &str) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO settings (key, value) VALUES ($1, $2) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            key,
            value
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MemorySettingsRepo {
    values: Mutex<std::collections::HashMap<String, String>>,
}

impl MemorySettingsRepo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, key: &str, value: &str) {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
    }
}

#[async_trait]
impl SettingsRepo for MemorySettingsRepo {
    async fn get(&self, key: &str) -> RepoResult<Option<String>> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    async fn set(&self, key: &str, value: &str) -> RepoResult<()> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[tokio::test]
    async fn returns_seeded_value() {
        let repo = MemorySettingsRepo::new();
        repo.seed("tipprunden_name", "Acme Tippspiel");
        assert_eq!(
            repo.get("tipprunden_name").await.unwrap().as_deref(),
            Some("Acme Tippspiel")
        );
    }

    #[tokio::test]
    async fn missing_key_returns_none() {
        let repo = MemorySettingsRepo::new();
        assert!(repo.get("absent").await.unwrap().is_none());
    }
}
