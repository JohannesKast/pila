//! Champion (Weltmeister) special-prediction persistence.

use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;

use super::{RepoError, RepoResult};

/// Champion view for the dashboard hero panel.
#[derive(Debug, Clone)]
pub struct ChampionView {
    pub team_name: String,
    pub flag_code: Option<String>,
}

/// One row of the "everyone's champion picks" table shown after lock.
#[derive(Debug, Clone)]
pub struct ChampionPickRow {
    pub user_name: String,
    pub champion_id: Option<i32>,
    pub team_name: Option<String>,
    pub flag_code: Option<String>,
}

#[async_trait]
pub trait SpecialPredictionRepo: Send + Sync {
    async fn get_user_champion(&self, user_id: Uuid) -> RepoResult<Option<i32>>;
    async fn upsert(&self, user_id: Uuid, champion_id: Option<i32>) -> RepoResult<()>;
    async fn list_with_user_names(&self) -> RepoResult<Vec<ChampionPickRow>>;
    async fn list_all_picks(&self) -> RepoResult<Vec<(Uuid, i32)>>;
    async fn user_champion_view(&self, user_id: Uuid) -> RepoResult<Option<ChampionView>>;
}

// ─── Postgres implementation ─────────────────────────────────────────────────

pub struct PgSpecialPredictionRepo {
    pool: PgPool,
}

impl PgSpecialPredictionRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SpecialPredictionRepo for PgSpecialPredictionRepo {
    async fn get_user_champion(&self, user_id: Uuid) -> RepoResult<Option<i32>> {
        let row = sqlx::query!(
            "SELECT champion_id FROM special_predictions WHERE user_id = $1",
            user_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row.and_then(|r| r.champion_id))
    }

    async fn upsert(&self, user_id: Uuid, champion_id: Option<i32>) -> RepoResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO special_predictions (user_id, champion_id, updated_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (user_id) DO UPDATE SET
                champion_id = EXCLUDED.champion_id,
                updated_at = NOW()
            "#,
            user_id,
            champion_id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn list_with_user_names(&self) -> RepoResult<Vec<ChampionPickRow>> {
        let rows = sqlx::query!(
            r#"
            SELECT u.name as user_name, sp.champion_id, t.name as "team_name?", t.flag_code
            FROM special_predictions sp
            JOIN users u ON u.id = sp.user_id
            LEFT JOIN teams t ON t.id = sp.champion_id
            ORDER BY u.name
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| ChampionPickRow {
                user_name: r.user_name,
                champion_id: r.champion_id,
                team_name: r.team_name,
                flag_code: r.flag_code,
            })
            .collect())
    }

    async fn list_all_picks(&self) -> RepoResult<Vec<(Uuid, i32)>> {
        let rows = sqlx::query!(
            "SELECT user_id, champion_id FROM special_predictions WHERE champion_id IS NOT NULL"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .filter_map(|r| r.champion_id.map(|c| (r.user_id, c)))
            .collect())
    }

    async fn user_champion_view(&self, user_id: Uuid) -> RepoResult<Option<ChampionView>> {
        let row = sqlx::query!(
            r#"
            SELECT t.name as "team_name!", t.flag_code
            FROM special_predictions sp
            JOIN teams t ON t.id = sp.champion_id
            WHERE sp.user_id = $1
            "#,
            user_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| ChampionView {
            team_name: r.team_name,
            flag_code: r.flag_code,
        }))
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MemorySpecialPredictionRepo {
    picks: Mutex<std::collections::HashMap<Uuid, Option<i32>>>,
}

impl MemorySpecialPredictionRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SpecialPredictionRepo for MemorySpecialPredictionRepo {
    async fn get_user_champion(&self, user_id: Uuid) -> RepoResult<Option<i32>> {
        Ok(self
            .picks
            .lock()
            .unwrap()
            .get(&user_id)
            .copied()
            .flatten())
    }

    async fn upsert(&self, user_id: Uuid, champion_id: Option<i32>) -> RepoResult<()> {
        self.picks.lock().unwrap().insert(user_id, champion_id);
        Ok(())
    }

    async fn list_with_user_names(&self) -> RepoResult<Vec<ChampionPickRow>> {
        // Fake doesn't have access to the user table — handler tests that
        // need the joined view should target the Postgres impl.
        Ok(Vec::new())
    }

    async fn list_all_picks(&self) -> RepoResult<Vec<(Uuid, i32)>> {
        Ok(self
            .picks
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(u, c)| c.map(|cid| (*u, cid)))
            .collect())
    }

    async fn user_champion_view(&self, _user_id: Uuid) -> RepoResult<Option<ChampionView>> {
        Ok(None)
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[tokio::test]
    async fn upsert_then_get_round_trips() {
        let repo = MemorySpecialPredictionRepo::new();
        let user = Uuid::new_v4();
        repo.upsert(user, Some(42)).await.unwrap();
        assert_eq!(repo.get_user_champion(user).await.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn upsert_overwrites_existing_pick() {
        let repo = MemorySpecialPredictionRepo::new();
        let user = Uuid::new_v4();
        repo.upsert(user, Some(1)).await.unwrap();
        repo.upsert(user, Some(2)).await.unwrap();
        assert_eq!(repo.get_user_champion(user).await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn upsert_none_clears_pick() {
        let repo = MemorySpecialPredictionRepo::new();
        let user = Uuid::new_v4();
        repo.upsert(user, Some(1)).await.unwrap();
        repo.upsert(user, None).await.unwrap();
        assert_eq!(repo.get_user_champion(user).await.unwrap(), None);
    }

    #[tokio::test]
    async fn list_all_picks_filters_none() {
        let repo = MemorySpecialPredictionRepo::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        repo.upsert(a, Some(11)).await.unwrap();
        repo.upsert(b, None).await.unwrap();
        let picks = repo.list_all_picks().await.unwrap();
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].1, 11);
    }
}
