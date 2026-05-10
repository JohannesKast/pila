//! Team persistence.

use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Mutex;

use super::{RepoError, RepoResult};

/// Team data needed by the champion-picker dropdown.
#[derive(Debug, Clone)]
pub struct TeamOption {
    pub id: i32,
    pub name: String,
    pub flag_code: Option<String>,
}

/// Payload for upserting one team coming back from the ESPN scoreboard.
/// Optional fields use `COALESCE` semantics so a later sync that omits e.g.
/// `flag_code` does not erase a value already populated.
#[derive(Debug, Clone)]
pub struct EspnTeamUpsert<'a> {
    pub espn_id: i32,
    pub name: &'a str,
    pub short_name: Option<&'a str>,
    pub flag_code: Option<&'a str>,
    pub group_letter: Option<&'a str>,
}

#[async_trait]
pub trait TeamRepo: Send + Sync {
    /// All "real" teams (excludes ESPN bracket placeholders such as
    /// "Group A Winner" or "Quarterfinal 2"). Sorted alphabetically.
    async fn list_real_for_dropdown(&self) -> RepoResult<Vec<TeamOption>>;

    /// True if a real (non-placeholder) team with the given id exists.
    async fn exists_real(&self, team_id: i32) -> RepoResult<bool>;

    /// Upsert one team. Idempotent on `id`.
    async fn upsert_from_espn(&self, upsert: EspnTeamUpsert<'_>) -> RepoResult<()>;
}

// ─── Postgres implementation ─────────────────────────────────────────────────

pub struct PgTeamRepo {
    pool: PgPool,
}

impl PgTeamRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TeamRepo for PgTeamRepo {
    async fn list_real_for_dropdown(&self) -> RepoResult<Vec<TeamOption>> {
        let rows = sqlx::query!(
            "SELECT id, name, flag_code FROM teams \
             WHERE name NOT LIKE 'Group %' \
               AND name NOT LIKE 'Quarterfinal %' \
               AND name NOT LIKE 'Semifinal %' \
               AND name NOT LIKE 'Round of %' \
               AND name NOT LIKE 'Third Place %' \
             ORDER BY name"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from)?;

        Ok(rows
            .into_iter()
            .map(|r| TeamOption {
                id: r.id,
                name: r.name,
                flag_code: r.flag_code,
            })
            .collect())
    }

    async fn exists_real(&self, team_id: i32) -> RepoResult<bool> {
        let row = sqlx::query_scalar!(
            "SELECT 1 AS dummy FROM teams \
             WHERE id = $1 \
               AND name NOT LIKE 'Group %' \
               AND name NOT LIKE 'Quarterfinal %' \
               AND name NOT LIKE 'Semifinal %' \
               AND name NOT LIKE 'Round of %' \
               AND name NOT LIKE 'Third Place %'",
            team_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(row.flatten().is_some())
    }

    async fn upsert_from_espn(&self, u: EspnTeamUpsert<'_>) -> RepoResult<()> {
        let short_name = u.short_name.map(|s| s.to_string());
        let flag_code = u.flag_code.map(|s| s.to_string());
        let group_for_team = u.group_letter.map(|g| {
            g.chars().next().unwrap_or(' ').to_string()
        });

        sqlx::query!(
            r#"
            INSERT INTO teams (id, name, short_name, flag_code, group_letter)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
              name = EXCLUDED.name,
              short_name = COALESCE(EXCLUDED.short_name, teams.short_name),
              flag_code = COALESCE(EXCLUDED.flag_code, teams.flag_code),
              group_letter = COALESCE(EXCLUDED.group_letter, teams.group_letter)
            "#,
            u.espn_id,
            u.name,
            short_name,
            flag_code,
            group_for_team,
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from)?;
        Ok(())
    }
}

// ─── In-memory fake ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MemoryTeamRepo {
    teams: Mutex<Vec<TeamOption>>,
}

impl MemoryTeamRepo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, team: TeamOption) {
        self.teams.lock().unwrap().push(team);
    }
}

fn is_placeholder(name: &str) -> bool {
    name.starts_with("Group ")
        || name.starts_with("Quarterfinal ")
        || name.starts_with("Semifinal ")
        || name.starts_with("Round of ")
        || name.starts_with("Third Place ")
}

#[async_trait]
impl TeamRepo for MemoryTeamRepo {
    async fn list_real_for_dropdown(&self) -> RepoResult<Vec<TeamOption>> {
        let mut out: Vec<TeamOption> = self
            .teams
            .lock()
            .unwrap()
            .iter()
            .filter(|t| !is_placeholder(&t.name))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn exists_real(&self, team_id: i32) -> RepoResult<bool> {
        Ok(self
            .teams
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.id == team_id && !is_placeholder(&t.name)))
    }

    async fn upsert_from_espn(&self, u: EspnTeamUpsert<'_>) -> RepoResult<()> {
        let mut teams = self.teams.lock().unwrap();
        let new_flag = u.flag_code.map(|s| s.to_string());
        if let Some(existing) = teams.iter_mut().find(|t| t.id == u.espn_id) {
            existing.name = u.name.to_string();
            if new_flag.is_some() {
                existing.flag_code = new_flag;
            }
        } else {
            teams.push(TeamOption {
                id: u.espn_id,
                name: u.name.to_string(),
                flag_code: new_flag,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    fn team(id: i32, name: &str) -> TeamOption {
        TeamOption {
            id,
            name: name.to_string(),
            flag_code: None,
        }
    }

    #[tokio::test]
    async fn list_excludes_placeholders_and_sorts_alphabetically() {
        let repo = MemoryTeamRepo::new();
        repo.seed(team(2, "Brazil"));
        repo.seed(team(1, "Argentina"));
        repo.seed(team(99, "Group A Winner"));
        repo.seed(team(98, "Quarterfinal 2"));
        repo.seed(team(3, "Canada"));

        let list = repo.list_real_for_dropdown().await.unwrap();
        let names: Vec<_> = list.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Argentina", "Brazil", "Canada"]);
    }

    #[tokio::test]
    async fn exists_real_rejects_placeholders() {
        let repo = MemoryTeamRepo::new();
        repo.seed(team(99, "Group A Winner"));
        repo.seed(team(11, "Germany"));
        assert!(repo.exists_real(11).await.unwrap());
        assert!(!repo.exists_real(99).await.unwrap());
        assert!(!repo.exists_real(404).await.unwrap());
    }
}
