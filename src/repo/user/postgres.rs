// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Postgres implementation of [`UserRepo`].

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{AdminUserRow, NewUser, UserAuth, UserBasic, UserFull, UserRepo};
use crate::repo::{RepoError, RepoResult};

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
            "SELECT id, name, is_admin, can_create_league, phone_number, email, jersey_preset, language, theme, league_id \
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
            theme: r.theme,
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
        let ids = sqlx::query_scalar!("SELECT id FROM users WHERE league_id = $1", league_id)
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
        sqlx::query!("UPDATE users SET is_admin = $1 WHERE id = $2", is_admin, id)
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
        sqlx::query!("UPDATE users SET language = $1 WHERE id = $2", language, id)
            .execute(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(())
    }

    async fn set_theme(&self, id: Uuid, theme: &str) -> RepoResult<()> {
        sqlx::query!("UPDATE users SET theme = $1 WHERE id = $2", theme, id)
            .execute(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(())
    }

    async fn set_email(&self, id: Uuid, email: Option<&str>) -> RepoResult<()> {
        sqlx::query!("UPDATE users SET email = $1 WHERE id = $2", email, id)
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
