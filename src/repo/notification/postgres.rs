// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Postgres implementation of [`NotificationRepo`].

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{user_id_for_db, ClosingSoonMatch, NotificationRepo};
use crate::notifier::{NotificationEvent, Notifier};
use crate::repo::{RepoError, RepoResult};
use crate::stage::Stage;

pub struct PgNotificationRepo {
    pool: PgPool,
}

impl PgNotificationRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NotificationRepo for PgNotificationRepo {
    async fn silence_existing_matches(&self, league_id: Uuid) -> RepoResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO sent_notifications (league_id, kind, ref_id, user_id)
            SELECT $1, 'match_closing_soon', m.id, '00000000-0000-0000-0000-000000000000'
            FROM matches m
            WHERE m.team_home_id IS NOT NULL AND m.team_away_id IS NOT NULL
            ON CONFLICT (league_id, kind, ref_id, user_id) DO NOTHING
            "#,
            league_id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn list_closing_soon_unnotified(
        &self,
        league_id: Uuid,
    ) -> RepoResult<Vec<ClosingSoonMatch>> {
        let rows = sqlx::query!(
            r#"
            SELECT m.id,
                   m.stage AS "stage: Stage",
                   m.group_letter,
                   m.kickoff_time AS "kickoff_time!",
                   th.name AS home,
                   ta.name AS away
            FROM matches m
            JOIN teams th ON th.id = m.team_home_id
            JOIN teams ta ON ta.id = m.team_away_id
            LEFT JOIN sent_notifications n
              ON n.kind = 'match_closing_soon' AND n.ref_id = m.id AND n.league_id = $1
            WHERE m.team_home_id IS NOT NULL AND m.team_away_id IS NOT NULL
              AND m.kickoff_time IS NOT NULL
              AND m.kickoff_time BETWEEN NOW() AND NOW() + INTERVAL '24 hours'
              AND n.ref_id IS NULL
            "#,
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| ClosingSoonMatch {
                match_id: r.id,
                stage: r.stage,
                group_letter: r.group_letter,
                kickoff_time: r.kickoff_time,
                home: r.home,
                away: r.away,
            })
            .collect())
    }

    async fn users_missing_prediction_for(
        &self,
        league_id: Uuid,
        match_id: i32,
    ) -> RepoResult<Vec<String>> {
        let names = sqlx::query_scalar!(
            r#"
            SELECT u.name FROM users u
            WHERE u.league_id = $1
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
        Ok(names)
    }

    async fn users_missing_champion(&self, league_id: Uuid) -> RepoResult<Vec<String>> {
        let names = sqlx::query_scalar!(
            r#"
            SELECT u.name FROM users u
            WHERE u.league_id = $1
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
        Ok(names)
    }

    async fn already_sent(
        &self,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
        user_id: Option<Uuid>,
    ) -> RepoResult<bool> {
        let uid = user_id_for_db(user_id);
        let row = sqlx::query!(
            "SELECT 1 AS dummy FROM sent_notifications \
             WHERE league_id = $1 AND kind = $2 AND ref_id = $3 AND user_id = $4",
            league_id,
            kind,
            ref_id,
            uid
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row.is_some())
    }

    async fn try_send(
        &self,
        notifier: &dyn Notifier,
        league_id: Uuid,
        kind: &str,
        ref_id: i32,
        user_id: Option<Uuid>,
        event: NotificationEvent,
    ) -> RepoResult<bool> {
        let uid = user_id_for_db(user_id);
        let mut tx = self.pool.begin().await.map_err(RepoError::from)?;

        let inserted = sqlx::query_scalar!(
            "INSERT INTO sent_notifications (league_id, kind, ref_id, user_id) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (league_id, kind, ref_id, user_id) DO NOTHING \
             RETURNING ref_id",
            league_id,
            kind,
            ref_id,
            uid
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(RepoError::from)?
        .is_some();

        if !inserted {
            // Already-sent path: drop the (effectively no-op) tx.
            let _ = tx.rollback().await;
            return Ok(false);
        }

        match notifier.notify(event).await {
            Ok(()) => {
                tx.commit().await.map_err(RepoError::from)?;
                tracing::info!(
                    "Sent notification: league={} {} {} user={:?}",
                    league_id,
                    kind,
                    ref_id,
                    user_id
                );
                Ok(true)
            }
            Err(e) => {
                tracing::error!(
                    "Notifier failed for league={} {} {} user={:?}: {:?}",
                    league_id,
                    kind,
                    ref_id,
                    user_id,
                    e
                );
                let _ = tx.rollback().await;
                Ok(false)
            }
        }
    }
}
