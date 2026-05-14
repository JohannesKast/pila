//! Match persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;

use super::{RepoError, RepoResult};
use crate::stage::Stage;

/// Row used to render the index page — every match the user might see, with
/// the user's own prediction merged in via LEFT JOIN.
#[derive(Debug, Clone)]
pub struct IndexMatchRow {
    pub id: i32,
    pub stage: Stage,
    pub group_letter: Option<String>,
    pub kickoff_time: Option<DateTime<Utc>>,
    pub status: String,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub team_home_id: Option<i32>,
    pub team_away_id: Option<i32>,
    pub home_name: String,
    pub away_name: String,
    pub home_flag: Option<String>,
    pub away_flag: Option<String>,
    pub predicted_home: Option<i32>,
    pub predicted_away: Option<i32>,
}

/// Information needed to validate a tip POST: when does the match start, and
/// are both teams known? Returned as `None` if the match id is unknown.
#[derive(Debug, Clone)]
pub struct MatchLockInfo {
    pub kickoff_time: Option<DateTime<Utc>>,
    pub team_home_id: Option<i32>,
    pub team_away_id: Option<i32>,
    pub stage: Stage,
}

/// One scored row of a finished group-stage match, joined with team display
/// data — the input to the standings calculator.
#[derive(Debug, Clone)]
pub struct FinishedGroupMatch {
    pub group_letter: String,
    pub home_id: i32,
    pub away_id: i32,
    pub score_home: i32,
    pub score_away: i32,
    pub home_name: String,
    pub home_flag: Option<String>,
    pub away_name: String,
    pub away_flag: Option<String>,
}

/// Payload for upserting one match coming back from the ESPN scoreboard.
/// All fields except `espn_event_id` and `stage` may flip back to `None`
/// across worker ticks (e.g. when ESPN walks back a TBD bracket slot), so
/// the SQL upsert uses `COALESCE` to preserve previously-seen values.
#[derive(Debug, Clone)]
pub struct EspnMatchUpsert<'a> {
    pub espn_event_id: i64,
    pub stage: Stage,
    pub group_letter: Option<&'a str>,
    pub team_home_id: Option<i32>,
    pub team_away_id: Option<i32>,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub kickoff_time: Option<DateTime<Utc>>,
    pub status: &'a str,
}

#[async_trait]
pub trait MatchRepo: Send + Sync {
    async fn list_for_index(&self, user_id: Uuid) -> RepoResult<Vec<IndexMatchRow>>;
    async fn find_lock_info(&self, match_id: i32) -> RepoResult<Option<MatchLockInfo>>;
    async fn first_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>>;
    async fn first_knockout_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>>;
    async fn actual_champion(&self) -> RepoResult<Option<i32>>;
    async fn finished_group_rows(&self) -> RepoResult<Vec<FinishedGroupMatch>>;
    /// Count of matches with both teams known whose kickoff has already
    /// passed — used as the denominator of the "Tippmoral" badge.
    async fn started_with_both_teams_count(
        &self,
        now: DateTime<Utc>,
    ) -> RepoResult<i64>;

    /// Upsert one match from the ESPN sync. Idempotent on `espn_event_id`.
    async fn upsert_from_espn(&self, upsert: EspnMatchUpsert<'_>) -> RepoResult<()>;

    /// Update match result and status (dev mode only).
    /// Used for simulating tournament progression.
    async fn update_result(
        &self,
        match_id: i32,
        score_home: Option<i32>,
        score_away: Option<i32>,
        status: &str,
    ) -> RepoResult<()>;
}

// ─── Postgres implementation ─────────────────────────────────────────────────

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
            "SELECT kickoff_time, team_home_id, team_away_id, stage as \"stage: Stage\" FROM matches WHERE id = $1",
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

        Ok(row.and_then(|r| {
            match (r.score_home, r.score_away, r.team_home_id, r.team_away_id) {
                (Some(sh), Some(sa), Some(hid), _) if sh > sa => Some(hid),
                (Some(sh), Some(sa), _, Some(aid)) if sa > sh => Some(aid),
                _ => None,
            }
        }))
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

    async fn started_with_both_teams_count(
        &self,
        now: DateTime<Utc>,
    ) -> RepoResult<i64> {
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

// ─── In-memory fake ──────────────────────────────────────────────────────────

/// Minimal seed type the fake stores; tests construct one directly.
#[derive(Debug, Clone, Default)]
pub struct FakeMatch {
    pub id: i32,
    pub stage: Stage,
    pub group_letter: Option<String>,
    pub kickoff_time: Option<DateTime<Utc>>,
    pub status: String,
    pub score_home: Option<i32>,
    pub score_away: Option<i32>,
    pub team_home_id: Option<i32>,
    pub team_away_id: Option<i32>,
    pub home_name: String,
    pub away_name: String,
    pub home_flag: Option<String>,
    pub away_flag: Option<String>,
}

impl FakeMatch {
    /// Sentinel match useful in handler tests (locked, unfinished, both teams set).
    pub fn locked_unfinished(id: i32, kickoff: DateTime<Utc>) -> Self {
        Self {
            id,
            stage: Stage::Group,
            group_letter: Some("A".into()),
            kickoff_time: Some(kickoff),
            status: "scheduled".into(),
            team_home_id: Some(1),
            team_away_id: Some(2),
            home_name: "TeamA".into(),
            away_name: "TeamB".into(),
            ..Default::default()
        }
    }
}

#[derive(Default)]
pub struct MemoryMatchRepo {
    matches: Mutex<Vec<FakeMatch>>,
    /// Map (user_id, match_id) → (predicted_home, predicted_away) so the index
    /// projection can mirror the LEFT JOIN.
    predictions: Mutex<std::collections::HashMap<(Uuid, i32), (i32, i32)>>,
}

impl MemoryMatchRepo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, m: FakeMatch) {
        self.matches.lock().unwrap().push(m);
    }

    /// Inject a prediction so `list_for_index` can mirror the LEFT JOIN.
    /// Used by tests that need to assert match × tip wiring.
    pub fn record_prediction(&self, user_id: Uuid, match_id: i32, h: i32, a: i32) {
        self.predictions
            .lock()
            .unwrap()
            .insert((user_id, match_id), (h, a));
    }
}

#[async_trait]
impl MatchRepo for MemoryMatchRepo {
    async fn list_for_index(&self, user_id: Uuid) -> RepoResult<Vec<IndexMatchRow>> {
        let matches = self.matches.lock().unwrap();
        let preds = self.predictions.lock().unwrap();
        let mut out: Vec<IndexMatchRow> = matches
            .iter()
            .map(|m| {
                let pred = preds.get(&(user_id, m.id)).copied();
                IndexMatchRow {
                    id: m.id,
                    stage: m.stage,
                    group_letter: m.group_letter.clone(),
                    kickoff_time: m.kickoff_time,
                    status: m.status.clone(),
                    score_home: m.score_home,
                    score_away: m.score_away,
                    team_home_id: m.team_home_id,
                    team_away_id: m.team_away_id,
                    home_name: m.home_name.clone(),
                    away_name: m.away_name.clone(),
                    home_flag: m.home_flag.clone(),
                    away_flag: m.away_flag.clone(),
                    predicted_home: pred.map(|p| p.0),
                    predicted_away: pred.map(|p| p.1),
                }
            })
            .collect();
        out.sort_by_key(|r| (stage_order(r.stage), r.kickoff_time, r.id));
        Ok(out)
    }

    async fn find_lock_info(&self, match_id: i32) -> RepoResult<Option<MatchLockInfo>> {
        Ok(self
            .matches
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.id == match_id)
            .map(|m| MatchLockInfo {
                kickoff_time: m.kickoff_time,
                team_home_id: m.team_home_id,
                team_away_id: m.team_away_id,
                stage: m.stage,
            }))
    }

    async fn first_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>> {
        Ok(self
            .matches
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| m.kickoff_time)
            .min())
    }

    async fn first_knockout_kickoff(&self) -> RepoResult<Option<DateTime<Utc>>> {
        Ok(self
            .matches
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.stage != Stage::Group)
            .filter_map(|m| m.kickoff_time)
            .min())
    }

    async fn actual_champion(&self) -> RepoResult<Option<i32>> {
        let matches = self.matches.lock().unwrap();
        let final_match = matches
            .iter()
            .filter(|m| m.stage == Stage::Final && m.status == "finished")
            .max_by_key(|m| m.kickoff_time);
        Ok(final_match.and_then(|m| {
            match (m.score_home, m.score_away, m.team_home_id, m.team_away_id) {
                (Some(sh), Some(sa), Some(hid), _) if sh > sa => Some(hid),
                (Some(sh), Some(sa), _, Some(aid)) if sa > sh => Some(aid),
                _ => None,
            }
        }))
    }

    async fn finished_group_rows(&self) -> RepoResult<Vec<FinishedGroupMatch>> {
        Ok(self
            .matches
            .lock()
            .unwrap()
            .iter()
            .filter(|m| {
                m.stage == Stage::Group
                    && m.status == "finished"
                    && m.group_letter.is_some()
                    && m.score_home.is_some()
                    && m.score_away.is_some()
                    && m.team_home_id.is_some()
                    && m.team_away_id.is_some()
            })
            .map(|m| FinishedGroupMatch {
                group_letter: m.group_letter.clone().unwrap(),
                home_id: m.team_home_id.unwrap(),
                away_id: m.team_away_id.unwrap(),
                score_home: m.score_home.unwrap(),
                score_away: m.score_away.unwrap(),
                home_name: m.home_name.clone(),
                home_flag: m.home_flag.clone(),
                away_name: m.away_name.clone(),
                away_flag: m.away_flag.clone(),
            })
            .collect())
    }

    async fn started_with_both_teams_count(
        &self,
        now: DateTime<Utc>,
    ) -> RepoResult<i64> {
        Ok(self
            .matches
            .lock()
            .unwrap()
            .iter()
            .filter(|m| {
                m.team_home_id.is_some()
                    && m.team_away_id.is_some()
                    && m.kickoff_time.is_some_and(|t| t < now)
            })
            .count() as i64)
    }

    async fn upsert_from_espn(&self, u: EspnMatchUpsert<'_>) -> RepoResult<()> {
        let mut matches = self.matches.lock().unwrap();
        let trimmed_letter = u
            .group_letter
            .map(|s| s.chars().next().unwrap_or(' ').to_string());

        // Look up by ESPN id first (the unique key); fall back to numeric id
        // for tests that pre-seed without an ESPN id.
        if let Some(existing) = matches
            .iter_mut()
            .find(|m| m.id as i64 == u.espn_event_id)
        {
            existing.stage = u.stage;
            existing.group_letter = trimmed_letter;
            existing.team_home_id = u.team_home_id.or(existing.team_home_id);
            existing.team_away_id = u.team_away_id.or(existing.team_away_id);
            existing.score_home = u.score_home.or(existing.score_home);
            existing.score_away = u.score_away.or(existing.score_away);
            existing.kickoff_time = u.kickoff_time.or(existing.kickoff_time);
            existing.status = u.status.to_string();
        } else {
            matches.push(FakeMatch {
                id: u.espn_event_id as i32,
                stage: u.stage,
                group_letter: trimmed_letter,
                kickoff_time: u.kickoff_time,
                status: u.status.to_string(),
                score_home: u.score_home,
                score_away: u.score_away,
                team_home_id: u.team_home_id,
                team_away_id: u.team_away_id,
                home_name: String::new(),
                away_name: String::new(),
                home_flag: None,
                away_flag: None,
            });
        }
        Ok(())
    }

    async fn update_result(
        &self,
        match_id: i32,
        score_home: Option<i32>,
        score_away: Option<i32>,
        status: &str,
    ) -> RepoResult<()> {
        let mut matches = self.matches.lock().unwrap();
        if let Some(m) = matches.iter_mut().find(|m| m.id == match_id) {
            m.score_home = score_home;
            m.score_away = score_away;
            m.status = status.to_string();
        }
        Ok(())
    }
}

fn stage_order(s: Stage) -> u8 {
    match s {
        Stage::Group => 0,
        Stage::RoundOf32 => 1,
        Stage::RoundOf16 => 2,
        Stage::QuarterFinal => 3,
        Stage::SemiFinal => 4,
        Stage::ThirdPlace => 5,
        Stage::Final => 6,
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(h: u32) -> DateTime<Utc> {
        // h is interpreted as "h hours after the WC kickoff" so callers can
        // freely use values >24 without crashing on invalid clock hours.
        Utc.with_ymd_and_hms(2026, 6, 11, 0, 0, 0).unwrap()
            + chrono::Duration::hours(h as i64)
    }

    #[tokio::test]
    async fn list_for_index_attaches_user_prediction() {
        let repo = MemoryMatchRepo::new();
        let user_id = Uuid::new_v4();
        let other_user = Uuid::new_v4();
        repo.seed(FakeMatch::locked_unfinished(1, ts(20)));
        repo.record_prediction(user_id, 1, 2, 1);
        repo.record_prediction(other_user, 1, 0, 0);

        let rows = repo.list_for_index(user_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].predicted_home, Some(2));
        assert_eq!(rows[0].predicted_away, Some(1));
    }

    #[tokio::test]
    async fn list_for_index_orders_by_stage_then_kickoff() {
        let repo = MemoryMatchRepo::new();
        let user_id = Uuid::new_v4();
        let mut a = FakeMatch::locked_unfinished(1, ts(20));
        a.stage = Stage::Final;
        let mut b = FakeMatch::locked_unfinished(2, ts(10));
        b.stage = Stage::Group;
        repo.seed(a);
        repo.seed(b);

        let rows = repo.list_for_index(user_id).await.unwrap();
        assert_eq!(rows[0].stage, Stage::Group);
        assert_eq!(rows[1].stage, Stage::Final);
    }

    #[tokio::test]
    async fn first_kickoff_returns_min() {
        let repo = MemoryMatchRepo::new();
        repo.seed(FakeMatch::locked_unfinished(1, ts(20)));
        repo.seed(FakeMatch::locked_unfinished(2, ts(15)));
        repo.seed(FakeMatch::locked_unfinished(3, ts(22)));
        assert_eq!(repo.first_kickoff().await.unwrap(), Some(ts(15)));
    }

    #[tokio::test]
    async fn actual_champion_picks_winner_of_finished_final() {
        let repo = MemoryMatchRepo::new();
        let mut m = FakeMatch::locked_unfinished(99, ts(20));
        m.stage = Stage::Final;
        m.status = "finished".into();
        m.score_home = Some(2);
        m.score_away = Some(1);
        m.team_home_id = Some(11);
        m.team_away_id = Some(22);
        repo.seed(m);
        assert_eq!(repo.actual_champion().await.unwrap(), Some(11));
    }

    #[tokio::test]
    async fn actual_champion_returns_none_on_draw() {
        let repo = MemoryMatchRepo::new();
        let mut m = FakeMatch::locked_unfinished(99, ts(20));
        m.stage = Stage::Final;
        m.status = "finished".into();
        m.score_home = Some(1);
        m.score_away = Some(1);
        repo.seed(m);
        assert_eq!(repo.actual_champion().await.unwrap(), None);
    }

    #[tokio::test]
    async fn started_count_excludes_tbd_and_future_matches() {
        let repo = MemoryMatchRepo::new();
        // started, both teams known
        repo.seed(FakeMatch::locked_unfinished(1, ts(10)));
        // future
        repo.seed(FakeMatch::locked_unfinished(2, ts(30)));
        // started, but TBD opponent
        let mut m = FakeMatch::locked_unfinished(3, ts(10));
        m.team_away_id = None;
        repo.seed(m);

        assert_eq!(
            repo.started_with_both_teams_count(ts(20)).await.unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn find_lock_info_returns_none_for_unknown_id() {
        let repo = MemoryMatchRepo::new();
        assert!(repo.find_lock_info(999).await.unwrap().is_none());
    }
}
