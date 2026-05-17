// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! League (Tipp-Liga) persistence.
//!
//! A league is the multi-tenancy boundary. Users belong to exactly one league,
//! and every aggregate query (leaderboard, badges, "other users' tips") must
//! filter by `league_id` so data never bleeds across leagues.
//!
//! Per-league configuration (Signal group, default language, RSS feed, ...)
//! lives in the `league_settings` key/value table. Adding a new setting means
//! adding a field to `LeagueConfig` and a key to `LeagueConfig::from_kv`,
//! never a migration.

use async_trait::async_trait;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use super::{RepoError, RepoResult};
use crate::scoring::MatchScoringSystem;

/// Stable UUID used by the test suite as a fixed league id. Production
/// leagues are created via the `/setup` flow or the super-admin "create
/// league" form — no league is seeded by migration.
pub const DEFAULT_LEAGUE_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000001);

/// Lightweight league row used by listings and the auth layer.
#[derive(Debug, Clone)]
pub struct League {
    pub id: Uuid,
    pub name: String,
    pub notifications_bootstrapped: bool,
}

/// Typed view over the `league_settings` key/value rows for a single league.
///
/// Adding a new per-league setting:
///   1. Add a field here.
///   2. Read its key in `from_kv` (with a sensible default).
///   3. Surface it in the admin settings form.
///
/// No migration needed — the table is intentionally schemaless.
#[derive(Debug, Clone)]
pub struct LeagueConfig {
    pub signal_group_id: Option<String>,
    pub signal_from_number: Option<String>,
    /// Locale code used as the fallback for newly invited users in this league.
    pub default_language: String,
    /// Optional RSS feed URL configured per league.
    pub rss_feed_url: Option<String>,
    /// When true, group-stage matches are hidden and cannot be tipped.
    pub predict_knockout_only: bool,
    /// Per-league match scoring system.
    pub match_scoring_system: MatchScoringSystem,
}

impl Default for LeagueConfig {
    fn default() -> Self {
        Self {
            signal_group_id: None,
            signal_from_number: None,
            default_language: "de".to_string(),
            rss_feed_url: None,
            predict_knockout_only: false,
            match_scoring_system: MatchScoringSystem::ExactScore,
        }
    }
}

impl LeagueConfig {
    pub const KEY_SIGNAL_GROUP_ID: &'static str = "signal_group_id";
    pub const KEY_SIGNAL_FROM_NUMBER: &'static str = "signal_from_number";
    pub const KEY_DEFAULT_LANGUAGE: &'static str = "default_language";
    pub const KEY_RSS_FEED_URL: &'static str = "rss_feed_url";
    pub const KEY_KO_ONLY: &'static str = "predict_knockout_only";
    pub const KEY_MATCH_SCORING_SYSTEM: &'static str = "match_scoring_system";

    fn from_kv(kv: HashMap<String, String>) -> Self {
        Self {
            signal_group_id: kv.get(Self::KEY_SIGNAL_GROUP_ID).cloned(),
            signal_from_number: kv.get(Self::KEY_SIGNAL_FROM_NUMBER).cloned(),
            default_language: kv
                .get(Self::KEY_DEFAULT_LANGUAGE)
                .cloned()
                .unwrap_or_else(|| "de".to_string()),
            rss_feed_url: kv.get(Self::KEY_RSS_FEED_URL).cloned(),
            predict_knockout_only: kv
                .get(Self::KEY_KO_ONLY)
                .map(|s| s == "true")
                .unwrap_or(false),
            match_scoring_system: kv
                .get(Self::KEY_MATCH_SCORING_SYSTEM)
                .and_then(|value| MatchScoringSystem::from_setting_value(value))
                .unwrap_or_default(),
        }
    }

    pub fn uses_winner_only_scoring(&self) -> bool {
        self.match_scoring_system.is_winner_only()
    }
}

#[async_trait]
pub trait LeagueRepo: Send + Sync {
    /// All leagues, sorted alphabetically by name.
    async fn list(&self) -> RepoResult<Vec<League>>;
    /// Returns `None` if no league with that id exists.
    async fn find_by_id(&self, id: Uuid) -> RepoResult<Option<League>>;
    /// Creates a new league with the given name and returns its id.
    async fn create(&self, name: &str) -> RepoResult<Uuid>;
    /// Marks the league's `notifications_bootstrapped` flag as true so
    /// the worker bootstrap pass does not repeat on the next restart.
    async fn set_bootstrapped(&self, league_id: Uuid) -> RepoResult<()>;
    /// Returns the league's typed config assembled from `league_settings`
    /// k/v rows. Missing keys fall back to `LeagueConfig` defaults.
    async fn get_config(&self, league_id: Uuid) -> RepoResult<LeagueConfig>;
    /// Set a single key/value pair. Passing `None` deletes the row.
    async fn set_setting(&self, league_id: Uuid, key: &str, value: Option<&str>) -> RepoResult<()>;
}

// ─── Postgres implementation ─────────────────────────────────────────────────

pub struct PgLeagueRepo {
    pool: PgPool,
}

impl PgLeagueRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LeagueRepo for PgLeagueRepo {
    async fn list(&self) -> RepoResult<Vec<League>> {
        let rows =
            sqlx::query!("SELECT id, name, notifications_bootstrapped FROM leagues ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| League {
                id: r.id,
                name: r.name,
                notifications_bootstrapped: r.notifications_bootstrapped,
            })
            .collect())
    }

    async fn find_by_id(&self, id: Uuid) -> RepoResult<Option<League>> {
        let row = sqlx::query!(
            "SELECT id, name, notifications_bootstrapped FROM leagues WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(row.map(|r| League {
            id: r.id,
            name: r.name,
            notifications_bootstrapped: r.notifications_bootstrapped,
        }))
    }

    async fn create(&self, name: &str) -> RepoResult<Uuid> {
        let row = sqlx::query!("INSERT INTO leagues (name) VALUES ($1) RETURNING id", name)
            .fetch_one(&self.pool)
            .await
            .map_err(RepoError::from)?;
        Ok(row.id)
    }

    async fn set_bootstrapped(&self, league_id: Uuid) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE leagues SET notifications_bootstrapped = TRUE WHERE id = $1",
            league_id
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }

    async fn get_config(&self, league_id: Uuid) -> RepoResult<LeagueConfig> {
        let rows = sqlx::query!(
            "SELECT key, value FROM league_settings WHERE league_id = $1",
            league_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        let kv: HashMap<String, String> = rows
            .into_iter()
            .filter_map(|r| r.value.map(|v| (r.key, v)))
            .collect();
        Ok(LeagueConfig::from_kv(kv))
    }

    async fn set_setting(&self, league_id: Uuid, key: &str, value: Option<&str>) -> RepoResult<()> {
        match value {
            Some(v) => {
                sqlx::query!(
                    "INSERT INTO league_settings (league_id, key, value) VALUES ($1, $2, $3) \
                     ON CONFLICT (league_id, key) DO UPDATE SET value = EXCLUDED.value",
                    league_id,
                    key,
                    v
                )
                .execute(&self.pool)
                .await
                .map_err(RepoError::from)?;
            }
            None => {
                sqlx::query!(
                    "DELETE FROM league_settings WHERE league_id = $1 AND key = $2",
                    league_id,
                    key
                )
                .execute(&self.pool)
                .await
                .map_err(RepoError::from)?;
            }
        }
        Ok(())
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

#[derive(Default)]
struct MemoryLeagueState {
    leagues: Vec<League>,
    settings: HashMap<(Uuid, String), String>,
}

#[derive(Default)]
pub struct MemoryLeagueRepo {
    inner: Mutex<MemoryLeagueState>,
}

impl MemoryLeagueRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper — seed a league directly without going through `create`.
    pub fn seed(&self, league: League) {
        self.inner.lock().unwrap().leagues.push(league);
    }
}

#[async_trait]
impl LeagueRepo for MemoryLeagueRepo {
    async fn list(&self) -> RepoResult<Vec<League>> {
        let mut leagues = self.inner.lock().unwrap().leagues.clone();
        leagues.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(leagues)
    }

    async fn find_by_id(&self, id: Uuid) -> RepoResult<Option<League>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .leagues
            .iter()
            .find(|l| l.id == id)
            .cloned())
    }

    async fn create(&self, name: &str) -> RepoResult<Uuid> {
        let id = Uuid::new_v4();
        self.inner.lock().unwrap().leagues.push(League {
            id,
            name: name.to_string(),
            notifications_bootstrapped: false,
        });
        Ok(id)
    }

    async fn set_bootstrapped(&self, league_id: Uuid) -> RepoResult<()> {
        if let Some(l) = self
            .inner
            .lock()
            .unwrap()
            .leagues
            .iter_mut()
            .find(|l| l.id == league_id)
        {
            l.notifications_bootstrapped = true;
        }
        Ok(())
    }

    async fn get_config(&self, league_id: Uuid) -> RepoResult<LeagueConfig> {
        let s = self.inner.lock().unwrap();
        let kv: HashMap<String, String> = s
            .settings
            .iter()
            .filter(|((lid, _), _)| *lid == league_id)
            .map(|((_, k), v)| (k.clone(), v.clone()))
            .collect();
        Ok(LeagueConfig::from_kv(kv))
    }

    async fn set_setting(&self, league_id: Uuid, key: &str, value: Option<&str>) -> RepoResult<()> {
        let mut s = self.inner.lock().unwrap();
        match value {
            Some(v) => {
                s.settings
                    .insert((league_id, key.to_string()), v.to_string());
            }
            None => {
                s.settings.remove(&(league_id, key.to_string()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[tokio::test]
    async fn create_then_list_returns_new_league() {
        let repo = MemoryLeagueRepo::new();
        let id = repo.create("Friends").await.unwrap();
        let list = repo.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].name, "Friends");
        assert!(!list[0].notifications_bootstrapped);
    }

    #[tokio::test]
    async fn list_is_alphabetical() {
        let repo = MemoryLeagueRepo::new();
        repo.create("Zoo").await.unwrap();
        repo.create("Acme").await.unwrap();
        repo.create("Mid").await.unwrap();
        let list = repo.list().await.unwrap();
        let names: Vec<_> = list.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["Acme", "Mid", "Zoo"]);
    }

    #[tokio::test]
    async fn find_by_id_returns_seeded_league() {
        let repo = MemoryLeagueRepo::new();
        let id = Uuid::new_v4();
        repo.seed(League {
            id,
            name: "Friends".into(),
            notifications_bootstrapped: false,
        });
        let found = repo.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(found.name, "Friends");
    }

    #[tokio::test]
    async fn set_bootstrapped_flips_flag() {
        let repo = MemoryLeagueRepo::new();
        let id = repo.create("Friends").await.unwrap();
        repo.set_bootstrapped(id).await.unwrap();
        let l = repo.find_by_id(id).await.unwrap().unwrap();
        assert!(l.notifications_bootstrapped);
    }

    #[tokio::test]
    async fn get_config_returns_defaults_when_empty() {
        let repo = MemoryLeagueRepo::new();
        let id = repo.create("Friends").await.unwrap();
        let cfg = repo.get_config(id).await.unwrap();
        assert_eq!(cfg.default_language, "de");
        assert!(cfg.signal_group_id.is_none());
        assert!(cfg.signal_from_number.is_none());
        assert!(cfg.rss_feed_url.is_none());
        assert!(!cfg.predict_knockout_only);
        assert_eq!(cfg.match_scoring_system, MatchScoringSystem::ExactScore);
    }

    #[tokio::test]
    async fn set_setting_then_get_config_round_trips() {
        let repo = MemoryLeagueRepo::new();
        let id = repo.create("Friends").await.unwrap();
        repo.set_setting(id, LeagueConfig::KEY_SIGNAL_GROUP_ID, Some("group.123"))
            .await
            .unwrap();
        repo.set_setting(id, LeagueConfig::KEY_DEFAULT_LANGUAGE, Some("en"))
            .await
            .unwrap();
        repo.set_setting(id, LeagueConfig::KEY_RSS_FEED_URL, Some("https://x/rss"))
            .await
            .unwrap();
        repo.set_setting(id, LeagueConfig::KEY_KO_ONLY, Some("true"))
            .await
            .unwrap();
        repo.set_setting(
            id,
            LeagueConfig::KEY_MATCH_SCORING_SYSTEM,
            Some(MatchScoringSystem::WINNER_ONLY_VALUE),
        )
        .await
        .unwrap();
        let cfg = repo.get_config(id).await.unwrap();
        assert_eq!(cfg.signal_group_id.as_deref(), Some("group.123"));
        assert_eq!(cfg.default_language, "en");
        assert_eq!(cfg.rss_feed_url.as_deref(), Some("https://x/rss"));
        assert!(cfg.predict_knockout_only);
        assert_eq!(cfg.match_scoring_system, MatchScoringSystem::WinnerOnly);
    }

    #[tokio::test]
    async fn set_setting_none_deletes_value() {
        let repo = MemoryLeagueRepo::new();
        let id = repo.create("Friends").await.unwrap();
        repo.set_setting(id, LeagueConfig::KEY_SIGNAL_GROUP_ID, Some("g"))
            .await
            .unwrap();
        repo.set_setting(id, LeagueConfig::KEY_SIGNAL_GROUP_ID, None)
            .await
            .unwrap();
        let cfg = repo.get_config(id).await.unwrap();
        assert!(cfg.signal_group_id.is_none());
    }

    #[tokio::test]
    async fn settings_isolated_between_leagues() {
        let repo = MemoryLeagueRepo::new();
        let a = repo.create("A").await.unwrap();
        let b = repo.create("B").await.unwrap();
        repo.set_setting(a, LeagueConfig::KEY_SIGNAL_GROUP_ID, Some("a-group"))
            .await
            .unwrap();
        let cfg_a = repo.get_config(a).await.unwrap();
        let cfg_b = repo.get_config(b).await.unwrap();
        assert_eq!(cfg_a.signal_group_id.as_deref(), Some("a-group"));
        assert!(cfg_b.signal_group_id.is_none());
    }
}
