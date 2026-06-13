// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Database abstraction layer.
//!
//! Each entity gets a trait (e.g. `UserRepo`) plus a Postgres implementation
//! (`PgUserRepo`) and an in-memory fake (`MemoryUserRepo`) for handler tests
//! that should not require a live database.
//!
//! Handlers depend only on the traits via `Arc<dyn …>` stored in `AppState`,
//! which means business logic is exercisable without a Postgres connection
//! and the boundary is the only place that sees raw SQL.

pub mod bootstrap;
pub mod fixture;
pub mod invite;
pub mod league;
pub mod notification;
pub mod prediction;
pub mod settings;
pub mod special_prediction;
pub mod team;
pub mod user;

pub use bootstrap::{BootstrapRepo, FirstLeagueParams, MemoryBootstrapRepo, PgBootstrapRepo};
pub use fixture::{MatchRepo, MemoryMatchRepo, PgMatchRepo};
pub use invite::{InviteRepo, MemoryInviteRepo, PgInviteRepo};
pub use league::{
    League, LeagueConfig, LeagueRepo, MemoryLeagueRepo, PgLeagueRepo, DEFAULT_LEAGUE_ID,
};
pub use notification::{MemoryNotificationRepo, NotificationRepo, PgNotificationRepo};
pub use prediction::{MemoryPredictionRepo, PgPredictionRepo, PredictionRepo};
pub use settings::{MemorySettingsRepo, PgSettingsRepo, SettingsRepo};
pub use special_prediction::{
    MemorySpecialPredictionRepo, PgSpecialPredictionRepo, SpecialPredictionRepo,
};
pub use team::{MemoryTeamRepo, PgTeamRepo, TeamRepo};
pub use user::{MemoryUserRepo, PgUserRepo, UserRepo};

use std::sync::Arc;

/// Bundle of every repo trait object the application needs.
///
/// `AppState` holds one of these so handlers can grab whichever repos they
/// need without long extractor lists or coupling to a specific backend.
#[derive(Clone)]
pub struct Repos {
    pub bootstrap: Arc<dyn BootstrapRepo>,
    pub users: Arc<dyn UserRepo>,
    pub leagues: Arc<dyn LeagueRepo>,
    pub invites: Arc<dyn InviteRepo>,
    pub matches: Arc<dyn MatchRepo>,
    pub predictions: Arc<dyn PredictionRepo>,
    pub special_predictions: Arc<dyn SpecialPredictionRepo>,
    pub teams: Arc<dyn TeamRepo>,
    pub settings: Arc<dyn SettingsRepo>,
    pub notifications: Arc<dyn NotificationRepo>,
}

impl Repos {
    /// Construct a `Repos` whose impls all talk to the given Postgres pool.
    pub fn from_pool(pool: sqlx::PgPool) -> Self {
        Self {
            bootstrap: Arc::new(PgBootstrapRepo::new(pool.clone())),
            users: Arc::new(PgUserRepo::new(pool.clone())),
            leagues: Arc::new(PgLeagueRepo::new(pool.clone())),
            invites: Arc::new(PgInviteRepo::new(pool.clone())),
            matches: Arc::new(PgMatchRepo::new(pool.clone())),
            predictions: Arc::new(PgPredictionRepo::new(pool.clone())),
            special_predictions: Arc::new(PgSpecialPredictionRepo::new(pool.clone())),
            teams: Arc::new(PgTeamRepo::new(pool.clone())),
            settings: Arc::new(PgSettingsRepo::new(pool.clone())),
            notifications: Arc::new(PgNotificationRepo::new(pool)),
        }
    }
}

/// Repository error. Kept narrow on purpose — handlers only need to know
/// "the persistence layer failed", not which SQL state code came back.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    /// A uniqueness constraint rejected the write — e.g. a duplicate display
    /// name in a league. Handlers surface this as a user-facing conflict
    /// instead of a generic 500.
    #[error("conflict: unique constraint violated")]
    Conflict,
}

pub type RepoResult<T> = Result<T, RepoError>;
