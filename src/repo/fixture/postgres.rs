// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Postgres implementation of [`MatchRepo`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{EspnMatchUpsert, FinishedGroupMatch, IndexMatchRow, MatchLockInfo, MatchRepo};
use crate::repo::{RepoError, RepoResult};
use crate::stage::Stage;

pub struct PgMatchRepo {
    pool: PgPool,
}

impl PgMatchRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MatchRepo for PgMatchRepo {
    async fn list_for_index(&self, user_id: Uuid) -> RepoResult<Vec<IndexMatchRow>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                m.id,
                m.stage as "stage: Stage",
                m.group_letter,
                m.kickoff_time,
                m.status,
                m.score_home as "score_home?",
                m.score_away as "score_away?",
                m.team_home_id as "team_home_id?",
                m.team_away_id as "team_away_id?",
                COALESCE(th.name, 'TBD') as "home_name!",
                COALESCE(ta.name, 'TBD') as "away_name!",
                th.flag_code as "home_flag?",
                ta.flag_code as "away_flag?",
                p.predicted_home as "predicted_home?",
                p.predicted_away as "predicted_away?"
            FROM matches m
            LEFT JOIN teams th ON th.id = m.team_home_id
            LEFT JOIN teams ta ON ta.id = m.team_away_id
            LEFT JOIN predictions p ON p.match_id = m.id AND p.user_id = $1
            ORDER BY
                CASE m.stage
                    WHEN 'group' THEN 0
                    WHEN 'round_of_32' THEN 1
                    WHEN 'round_of_16' THEN 2
                    WHEN 'quarter_final' THEN 3
                    WHEN 'semi_final' THEN 4
                    WHEN 'third_place' THEN 5
                    WHEN 'final' THEN 6
                END,
                m.kickoff_time NULLS LAST,
                m.id
            "#,
            user_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| IndexMatchRow {
                id: r.id,
                stage: r.stage,
                group_letter: r.group_letter,
                kickoff_time: r.kickoff_time,
                status: r.status,
                score_home: r.score_home,
                score_away: r.score_away,
                team_home_id: r.team_home_id,
                team_away_id: r.team_away_id,
                home_name: r.home_name,
                away_name: r.away_name,
                home_flag: r.home_flag,
                away_flag: r.away_flag,
                predicted_home: r.predicted_home,
                predicted_away: r.predicted_away,
            })
            .collect())
    }

    async fn find_lock_info(&self, match_id: i32) -> RepoResult<Option<MatchLockInfo>> {
        let row = sqlx::query!(
            r#"
            SELECT m.kickoff_time,
                   m.team_home_id as "team_home_id?",
                   m.team_away_id as "team_away_id?",
                   m.stage as "stage: Stage",
                   COALESCE(th.name, 'TBD') as "home_name!",
                   COALESCE(ta.name, 'TBD') as "away_name!",
                   th.flag_code as "home_flag_code?",
                   ta.flag_code as "away_flag_code?"
            FROM matches m
            LEFT JOIN teams th ON th.id = m.team_home_id
            LEFT JOIN teams ta ON ta.id = m.team_away_id
            WHERE m.id = $1
            "#,
            match_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| MatchLockInfo {
            kickoff_time: r.kickoff_time,
            team_home_id: r.team_home_id,
            team_away_id: r.team_away_id,
            stage: r.stage,
            home_name: r.home_name,
            away_name: r.away_name,
            home_flag_code: r.home_flag_code,
            away_flag_code: r.away_flag_code,
        }))
    }

    async fn first_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>> {
        let v = sqlx::query_scalar!("SELECT MIN(kickoff_time) FROM matches")
            .fetch_one(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(v)
    }

    async fn first_knockout_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>> {
        let v = sqlx::query_scalar!(
            "SELECT MIN(kickoff_time) FROM matches WHERE stage != 'group'::match_stage"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(v)
    }

    async fn actual_champion(&self) -> RepoResult<Option<i32>> {
        let row = sqlx::query!(
            r#"
            SELECT team_home_id as "team_home_id?",
                   team_away_id as "team_away_id?",
                   score_home   as "score_home?",
                   score_away   as "score_away?",
                   status
            FROM matches
            WHERE stage = 'final'::match_stage AND status = 'finished'
            ORDER BY kickoff_time DESC
            LIMIT 1
            "#
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.and_then(
            |r| match (r.score_home, r.score_away, r.team_home_id, r.team_away_id) {
                (Some(sh), Some(sa), Some(hid), _) if sh > sa => Some(hid),
                (Some(sh), Some(sa), _, Some(aid)) if sa > sh => Some(aid),
                _ => None,
            },
        ))
    }

    async fn finished_group_rows(&self) -> RepoResult<Vec<FinishedGroupMatch>> {
        let rows = sqlx::query!(
            r#"
            SELECT m.group_letter as "letter!",
                   m.team_home_id as "home_id!",
                   m.team_away_id as "away_id!",
                   m.score_home   as "score_home!",
                   m.score_away   as "score_away!",
                   th.name        as "home_name!",
                   th.flag_code   as "home_flag?",
                   ta.name        as "away_name!",
                   ta.flag_code   as "away_flag?"
            FROM matches m
            JOIN teams th ON th.id = m.team_home_id
            JOIN teams ta ON ta.id = m.team_away_id
            WHERE m.stage = 'group'::match_stage
              AND m.status = 'finished'
              AND m.group_letter IS NOT NULL
              AND m.score_home IS NOT NULL
              AND m.score_away IS NOT NULL
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| FinishedGroupMatch {
                group_letter: r.letter,
                home_id: r.home_id,
                away_id: r.away_id,
                score_home: r.score_home,
                score_away: r.score_away,
                home_name: r.home_name,
                home_flag: r.home_flag,
                away_name: r.away_name,
                away_flag: r.away_flag,
            })
            .collect())
    }

    async fn started_with_both_teams_count(&self, now: DateTime<Utc>) -> RepoResult<i64> {
        let c = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "c!"
            FROM matches
            WHERE kickoff_time IS NOT NULL
              AND kickoff_time < $1
              AND team_home_id IS NOT NULL
              AND team_away_id IS NOT NULL
            "#,
            now
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(c)
    }

    async fn upsert_from_espn(&self, u: EspnMatchUpsert<'_>) -> RepoResult<()> {
        // Trim to the single-letter group representation expected by the
        // schema's CHECK constraint; mirrors the previous worker behaviour.
        let group_letter = u
            .group_letter
            .map(|s| s.chars().next().unwrap_or(' ').to_string());

        sqlx::query!(
            r#"
            INSERT INTO matches (espn_event_id, stage, group_letter, team_home_id, team_away_id,
                                 score_home, score_away, kickoff_time, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (espn_event_id) DO UPDATE SET
              stage = EXCLUDED.stage,
              group_letter = EXCLUDED.group_letter,
              team_home_id = COALESCE(EXCLUDED.team_home_id, matches.team_home_id),
              team_away_id = COALESCE(EXCLUDED.team_away_id, matches.team_away_id),
              score_home = COALESCE(EXCLUDED.score_home, matches.score_home),
              score_away = COALESCE(EXCLUDED.score_away, matches.score_away),
              kickoff_time = COALESCE(EXCLUDED.kickoff_time, matches.kickoff_time),
              status = EXCLUDED.status
            "#,
            u.espn_event_id,
            u.stage as Stage,
            group_letter,
            u.team_home_id,
            u.team_away_id,
            u.score_home,
            u.score_away,
            u.kickoff_time,
            u.status,
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn update_result(
        &self,
        match_id: i32,
        score_home: Option<i32>,
        score_away: Option<i32>,
        status: &str,
    ) -> RepoResult<()> {
        sqlx::query!(
            r#"
            UPDATE matches
            SET score_home = $1, score_away = $2, status = $3
            WHERE id = $4
            "#,
            score_home,
            score_away,
            status,
            match_id,
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }
}
