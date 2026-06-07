// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Postgres implementation of [`InviteRepo`].

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{InviteLink, InviteRepo};
use crate::repo::{RepoError, RepoResult};

pub struct PgInviteRepo {
    pool: PgPool,
}

impl PgInviteRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteRepo for PgInviteRepo {
    async fn create(&self, league_id: Uuid, token: &str, label: Option<&str>) -> RepoResult<Uuid> {
        let row = sqlx::query!(
            "INSERT INTO invite_links (league_id, token, label) VALUES ($1, $2, $3) RETURNING id",
            league_id,
            token,
            label
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row.id)
    }

    async fn list_for_league(&self, league_id: Uuid) -> RepoResult<Vec<InviteLink>> {
        let rows = sqlx::query!(
            "SELECT id, league_id, token, label, created_at \
             FROM invite_links WHERE league_id = $1 ORDER BY created_at DESC",
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| InviteLink {
                id: r.id,
                league_id: r.league_id,
                token: r.token,
                label: r.label,
                created_at: r.created_at,
            })
            .collect())
    }

    async fn find_by_token(&self, token: &str) -> RepoResult<Option<InviteLink>> {
        let row = sqlx::query!(
            "SELECT id, league_id, token, label, created_at FROM invite_links WHERE token = $1",
            token
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| InviteLink {
            id: r.id,
            league_id: r.league_id,
            token: r.token,
            label: r.label,
            created_at: r.created_at,
        }))
    }

    async fn find_by_id(&self, id: Uuid) -> RepoResult<Option<InviteLink>> {
        let row = sqlx::query!(
            "SELECT id, league_id, token, label, created_at FROM invite_links WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| InviteLink {
            id: r.id,
            league_id: r.league_id,
            token: r.token,
            label: r.label,
            created_at: r.created_at,
        }))
    }

    async fn delete(&self, id: Uuid) -> RepoResult<()> {
        sqlx::query!("DELETE FROM invite_links WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(())
    }
}
