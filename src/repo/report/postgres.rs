// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

use async_trait::async_trait;
use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::{MatchdayReport, MatchdayReportRepo};
use crate::repo::{RepoError, RepoResult};

pub struct PgMatchdayReportRepo {
    pool: PgPool,
}

impl PgMatchdayReportRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MatchdayReportRepo for PgMatchdayReportRepo {
    async fn get(&self, league_id: Uuid, date: NaiveDate) -> RepoResult<Option<MatchdayReport>> {
        let row = sqlx::query!(
            "SELECT league_id, matchday_date, language, content, model, generated_at \
             FROM ai_matchday_reports WHERE league_id = $1 AND matchday_date = $2",
            league_id,
            date
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| MatchdayReport {
            league_id: r.league_id,
            matchday_date: r.matchday_date,
            language: r.language,
            content: r.content,
            model: r.model,
            generated_at: r.generated_at,
        }))
    }

    async fn exists(&self, league_id: Uuid, date: NaiveDate) -> RepoResult<bool> {
        let row = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM ai_matchday_reports \
             WHERE league_id = $1 AND matchday_date = $2)",
            league_id,
            date
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row.unwrap_or(false))
    }

    async fn latest_date(&self, league_id: Uuid) -> RepoResult<Option<NaiveDate>> {
        let row = sqlx::query_scalar!(
            "SELECT MAX(matchday_date) FROM ai_matchday_reports WHERE league_id = $1",
            league_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row)
    }

    async fn neighbors(
        &self,
        league_id: Uuid,
        date: NaiveDate,
    ) -> RepoResult<(Option<NaiveDate>, Option<NaiveDate>)> {
        let older = sqlx::query_scalar!(
            "SELECT MAX(matchday_date) FROM ai_matchday_reports \
             WHERE league_id = $1 AND matchday_date < $2",
            league_id,
            date
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;

        let newer = sqlx::query_scalar!(
            "SELECT MIN(matchday_date) FROM ai_matchday_reports \
             WHERE league_id = $1 AND matchday_date > $2",
            league_id,
            date
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok((older, newer))
    }

    async fn insert(&self, report: &MatchdayReport) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO ai_matchday_reports \
                (league_id, matchday_date, language, content, model, generated_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (league_id, matchday_date) DO NOTHING",
            report.league_id,
            report.matchday_date,
            report.language,
            report.content,
            report.model,
            report.generated_at
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }
}
