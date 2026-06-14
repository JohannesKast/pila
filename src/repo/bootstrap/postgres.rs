// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

use async_trait::async_trait;
use sqlx::PgPool;

use super::{BootstrapRepo, FirstLeagueParams};
use crate::repo::{RepoError, RepoResult};

pub struct PgBootstrapRepo {
    pool: PgPool,
}

impl PgBootstrapRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BootstrapRepo for PgBootstrapRepo {
    async fn create_first_league_and_admin(
        &self,
        params: FirstLeagueParams<'_>,
    ) -> RepoResult<uuid::Uuid> {
        let mut tx = self.pool.begin().await.map_err(RepoError::from)?;

        let league_id: uuid::Uuid = sqlx::query_scalar!(
            "INSERT INTO leagues (name) VALUES ($1) RETURNING id",
            params.league_name
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(RepoError::from)?;

        for (key, value) in params.settings {
            if value.is_empty() {
                continue;
            }
            sqlx::query!(
                "INSERT INTO league_settings (league_id, key, value) VALUES ($1, $2, $3) \
                 ON CONFLICT (league_id, key) DO UPDATE SET value = EXCLUDED.value",
                league_id,
                key,
                value
            )
            .execute(&mut *tx)
            .await
            .map_err(RepoError::from)?;
        }

        sqlx::query!(
            "INSERT INTO users (id, name, token, is_admin, phone_number, email, league_id, language) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            params.user_id,
            params.user_name,
            params.token,
            true,
            params.phone_number,
            params.email,
            league_id,
            params.language
        )
        .execute(&mut *tx)
        .await
        .map_err(RepoError::from)?;

        sqlx::query!(
            "UPDATE users SET can_create_league = TRUE WHERE id = $1",
            params.user_id
        )
        .execute(&mut *tx)
        .await
        .map_err(RepoError::from)?;

        tx.commit().await.map_err(RepoError::from)?;
        Ok(league_id)
    }
}
