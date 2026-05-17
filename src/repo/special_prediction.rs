// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

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
    /// Returns the user's current champion pick (team id), or `None` if they
    /// have not made one yet.
    async fn get_user_champion(&self, user_id: Uuid) -> RepoResult<Option<i32>>;
    /// Insert or overwrite the user's champion pick. Pass `None` for
    /// `champion_id` to clear the pick.
    async fn upsert(&self, user_id: Uuid, champion_id: Option<i32>) -> RepoResult<()>;
    /// Champion picks of users in the given league, joined with user/team names.
    async fn list_with_user_names(&self, league_id: Uuid) -> RepoResult<Vec<ChampionPickRow>>;
    /// All `(user_id, champion_team_id)` pairs in the given league — used by
    /// the badge engine to compute champion-related stats.
    async fn list_all_picks(&self, league_id: Uuid) -> RepoResult<Vec<(Uuid, i32)>>;
    /// Display name and flag code for the user's champion pick. Returns `None`
    /// if the user has no pick or the team id does not resolve to a known team.
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

    async fn list_with_user_names(&self, league_id: Uuid) -> RepoResult<Vec<ChampionPickRow>> {
        let rows = sqlx::query!(
            r#"
            SELECT u.name as user_name, sp.champion_id, t.name as "team_name?", t.flag_code
            FROM special_predictions sp
            JOIN users u ON u.id = sp.user_id
            LEFT JOIN teams t ON t.id = sp.champion_id
            WHERE u.league_id = $1
            ORDER BY u.name
            "#,
            league_id
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

    async fn list_all_picks(&self, league_id: Uuid) -> RepoResult<Vec<(Uuid, i32)>> {
        let rows = sqlx::query!(
            "SELECT sp.user_id, sp.champion_id FROM special_predictions sp \
             JOIN users u ON u.id = sp.user_id \
             WHERE sp.champion_id IS NOT NULL AND u.league_id = $1",
            league_id
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
struct MemorySpecialState {
    /// (user_id, league_id) → optional champion_id. League is stored alongside
    /// the pick so the fake can answer league-scoped queries without needing
    /// a separate user repo.
    picks: std::collections::HashMap<Uuid, (Uuid, Option<i32>)>,
    /// user_id → display name (test seed). When set, `list_with_user_names`
    /// joins through this map.
    user_names: std::collections::HashMap<Uuid, String>,
}

#[derive(Default)]
pub struct MemorySpecialPredictionRepo {
    inner: Mutex<MemorySpecialState>,
}

impl MemorySpecialPredictionRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper — record `user_id`'s league + display name so league-scoped
    /// queries can resolve the join. `upsert` does not learn the league_id on
    /// its own, so tests calling `list_with_user_names` must seed first.
    pub fn seed_user(&self, user_id: Uuid, league_id: Uuid, name: &str) {
        let mut s = self.inner.lock().unwrap();
        s.picks.entry(user_id).or_insert((league_id, None));
        s.user_names.insert(user_id, name.to_string());
    }
}

#[async_trait]
impl SpecialPredictionRepo for MemorySpecialPredictionRepo {
    async fn get_user_champion(&self, user_id: Uuid) -> RepoResult<Option<i32>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .picks
            .get(&user_id)
            .and_then(|(_, c)| *c))
    }

    async fn upsert(&self, user_id: Uuid, champion_id: Option<i32>) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        s.picks
            .entry(user_id)
            .and_modify(|e| e.1 = champion_id)
            .or_insert((Uuid::nil(), champion_id));
        Ok(())
    }

    async fn list_with_user_names(&self, league_id: Uuid) -> RepoResult<Vec<ChampionPickRow>> {
        let s = self.inner.lock().unwrap();
        let mut rows: Vec<ChampionPickRow> = s
            .picks
            .iter()
            .filter(|(_, (lid, _))| *lid == league_id)
            .filter_map(|(uid, (_, cid))| {
                s.user_names.get(uid).map(|name| ChampionPickRow {
                    user_name: name.clone(),
                    champion_id: *cid,
                    team_name: None,
                    flag_code: None,
                })
            })
            .collect();
        rows.sort_by(|a, b| a.user_name.cmp(&b.user_name));
        Ok(rows)
    }

    async fn list_all_picks(&self, league_id: Uuid) -> RepoResult<Vec<(Uuid, i32)>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .picks
            .iter()
            .filter(|(_, (lid, _))| *lid == league_id)
            .filter_map(|(u, (_, c))| c.map(|cid| (*u, cid)))
            .collect())
    }

    async fn user_champion_view(&self, _user_id: Uuid) -> RepoResult<Option<ChampionView>> {
        Ok(None)
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::repo::DEFAULT_LEAGUE_ID;

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
    async fn list_all_picks_filters_none_and_scopes_by_league() {
        let repo = MemorySpecialPredictionRepo::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        repo.seed_user(a, DEFAULT_LEAGUE_ID, "A");
        repo.seed_user(b, DEFAULT_LEAGUE_ID, "B");
        repo.upsert(a, Some(11)).await.unwrap();
        repo.upsert(b, None).await.unwrap();
        let picks = repo.list_all_picks(DEFAULT_LEAGUE_ID).await.unwrap();
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].1, 11);
    }

    #[tokio::test]
    async fn list_all_picks_excludes_other_leagues() {
        let repo = MemorySpecialPredictionRepo::new();
        let other = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        repo.seed_user(a, DEFAULT_LEAGUE_ID, "A");
        repo.seed_user(b, other, "B");
        repo.upsert(a, Some(1)).await.unwrap();
        repo.upsert(b, Some(2)).await.unwrap();
        let default_picks = repo.list_all_picks(DEFAULT_LEAGUE_ID).await.unwrap();
        assert_eq!(default_picks, vec![(a, 1)]);
        let other_picks = repo.list_all_picks(other).await.unwrap();
        assert_eq!(other_picks, vec![(b, 2)]);
    }
}
