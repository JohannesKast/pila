// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! In-memory [`InviteRepo`] fake for tests.

use async_trait::async_trait;
use chrono::Utc;
use std::sync::Mutex;
use uuid::Uuid;

use super::{InviteLink, InviteRepo};
use crate::repo::RepoResult;

#[derive(Default)]
pub struct MemoryInviteRepo {
    inner: Mutex<Vec<InviteLink>>,
}

impl MemoryInviteRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl InviteRepo for MemoryInviteRepo {
    async fn create(&self, league_id: Uuid, token: &str, label: Option<&str>) -> RepoResult<Uuid> {
        let id = Uuid::new_v4();
        self.inner.lock().unwrap().push(InviteLink {
            id,
            league_id,
            token: token.to_string(),
            label: label.map(|s| s.to_string()),
            created_at: Utc::now(),
        });
        Ok(id)
    }

    async fn list_for_league(&self, league_id: Uuid) -> RepoResult<Vec<InviteLink>> {
        let mut rows: Vec<InviteLink> = self
            .inner
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.league_id == league_id)
            .cloned()
            .collect();
        // Newest first, mirroring the Postgres ORDER BY.
        rows.sort_by_key(|l| std::cmp::Reverse(l.created_at));
        Ok(rows)
    }

    async fn find_by_token(&self, token: &str) -> RepoResult<Option<InviteLink>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .find(|l| l.token == token)
            .cloned())
    }

    async fn find_by_id(&self, id: Uuid) -> RepoResult<Option<InviteLink>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .find(|l| l.id == id)
            .cloned())
    }

    async fn delete(&self, id: Uuid) -> RepoResult<()> {
        self.inner.lock().unwrap().retain(|l| l.id != id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_then_find_by_token_round_trips() {
        let repo = MemoryInviteRepo::new();
        let league = Uuid::new_v4();
        repo.create(league, "tok-1", Some("Office")).await.unwrap();
        let found = repo.find_by_token("tok-1").await.unwrap().unwrap();
        assert_eq!(found.league_id, league);
        assert_eq!(found.label.as_deref(), Some("Office"));
    }

    #[tokio::test]
    async fn list_is_scoped_to_league() {
        let repo = MemoryInviteRepo::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        repo.create(a, "a1", None).await.unwrap();
        repo.create(a, "a2", None).await.unwrap();
        repo.create(b, "b1", None).await.unwrap();
        assert_eq!(repo.list_for_league(a).await.unwrap().len(), 2);
        assert_eq!(repo.list_for_league(b).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_revokes_token() {
        let repo = MemoryInviteRepo::new();
        let league = Uuid::new_v4();
        let id = repo.create(league, "tok", None).await.unwrap();
        repo.delete(id).await.unwrap();
        assert!(repo.find_by_token("tok").await.unwrap().is_none());
    }
}
