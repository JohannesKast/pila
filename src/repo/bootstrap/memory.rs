// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

use async_trait::async_trait;

use super::{BootstrapRepo, FirstLeagueParams};
use crate::repo::RepoResult;

#[derive(Default)]
pub struct MemoryBootstrapRepo;

impl MemoryBootstrapRepo {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BootstrapRepo for MemoryBootstrapRepo {
    async fn create_first_league_and_admin(
        &self,
        _params: FirstLeagueParams<'_>,
    ) -> RepoResult<()> {
        Ok(())
    }
}
