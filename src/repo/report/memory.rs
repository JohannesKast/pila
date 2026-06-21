// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

use async_trait::async_trait;
use chrono::NaiveDate;
use std::sync::Mutex;
use uuid::Uuid;

use super::{MatchdayReport, MatchdayReportRepo};
use crate::repo::RepoResult;

#[derive(Default)]
pub struct MemoryMatchdayReportRepo {
    inner: Mutex<Vec<MatchdayReport>>,
}

impl MemoryMatchdayReportRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MatchdayReportRepo for MemoryMatchdayReportRepo {
    async fn get(&self, league_id: Uuid, date: NaiveDate) -> RepoResult<Option<MatchdayReport>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.league_id == league_id && r.matchday_date == date)
            .cloned())
    }

    async fn exists(&self, league_id: Uuid, date: NaiveDate) -> RepoResult<bool> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.league_id == league_id && r.matchday_date == date))
    }

    async fn latest_date(&self, league_id: Uuid) -> RepoResult<Option<NaiveDate>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.league_id == league_id)
            .map(|r| r.matchday_date)
            .max())
    }

    async fn neighbors(
        &self,
        league_id: Uuid,
        date: NaiveDate,
    ) -> RepoResult<(Option<NaiveDate>, Option<NaiveDate>)> {
        let g = self.inner.lock().unwrap();
        let older = g
            .iter()
            .filter(|r| r.league_id == league_id && r.matchday_date < date)
            .map(|r| r.matchday_date)
            .max();
        let newer = g
            .iter()
            .filter(|r| r.league_id == league_id && r.matchday_date > date)
            .map(|r| r.matchday_date)
            .min();
        Ok((older, newer))
    }

    async fn insert(&self, report: &MatchdayReport) -> RepoResult<()> {
        let mut g = self.inner.lock().unwrap();
        let dup = g
            .iter()
            .any(|r| r.league_id == report.league_id && r.matchday_date == report.matchday_date);
        if !dup {
            g.push(report.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn report(league: Uuid, date: NaiveDate) -> MatchdayReport {
        MatchdayReport {
            league_id: league,
            matchday_date: date,
            language: "de".into(),
            content: format!("Recap for {date}"),
            model: "gemini::gemini-2.5-flash".into(),
            generated_at: Utc::now(),
        }
    }

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, day).unwrap()
    }

    #[tokio::test]
    async fn insert_is_idempotent_per_league_and_date() {
        let repo = MemoryMatchdayReportRepo::new();
        let league = Uuid::new_v4();
        repo.insert(&report(league, d(11))).await.unwrap();
        repo.insert(&report(league, d(11))).await.unwrap();
        assert!(repo.exists(league, d(11)).await.unwrap());
        assert_eq!(repo.latest_date(league).await.unwrap(), Some(d(11)));
    }

    #[tokio::test]
    async fn latest_and_neighbors_track_navigation() {
        let repo = MemoryMatchdayReportRepo::new();
        let league = Uuid::new_v4();
        for day in [11u32, 13, 15] {
            repo.insert(&report(league, d(day))).await.unwrap();
        }
        assert_eq!(repo.latest_date(league).await.unwrap(), Some(d(15)));
        // Middle day: older = 11, newer = 15.
        assert_eq!(
            repo.neighbors(league, d(13)).await.unwrap(),
            (Some(d(11)), Some(d(15)))
        );
        // Newest day: no newer neighbour.
        assert_eq!(
            repo.neighbors(league, d(15)).await.unwrap(),
            (Some(d(13)), None)
        );
        // Oldest day: no older neighbour.
        assert_eq!(
            repo.neighbors(league, d(11)).await.unwrap(),
            (None, Some(d(13)))
        );
    }

    #[tokio::test]
    async fn reports_are_league_scoped() {
        let repo = MemoryMatchdayReportRepo::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        repo.insert(&report(a, d(11))).await.unwrap();
        assert!(!repo.exists(b, d(11)).await.unwrap());
        assert_eq!(repo.latest_date(b).await.unwrap(), None);
    }
}
