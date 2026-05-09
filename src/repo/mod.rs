//! Database abstraction layer.
//!
//! Each entity gets a trait (e.g. `UserRepo`) plus a Postgres implementation
//! (`PgUserRepo`) and an in-memory fake (`MemoryUserRepo`) for handler tests
//! that should not require a live database.
//!
//! Handlers depend only on the traits via `Arc<dyn …>` stored in `AppState`,
//! which means business logic is exercisable without a Postgres connection
//! and the boundary is the only place that sees raw SQL.

pub mod match_;
pub mod prediction;
pub mod settings;
pub mod special_prediction;
pub mod team;
pub mod user;

pub use match_::{MatchRepo, PgMatchRepo};
pub use prediction::{PgPredictionRepo, PredictionRepo};
pub use settings::{PgSettingsRepo, SettingsRepo};
pub use special_prediction::{PgSpecialPredictionRepo, SpecialPredictionRepo};
pub use team::{PgTeamRepo, TeamRepo};
pub use user::{PgUserRepo, UserRepo};

use std::sync::Arc;

/// Bundle of every repo trait object the application needs.
///
/// `AppState` holds one of these so handlers can grab whichever repos they
/// need without long extractor lists or coupling to a specific backend.
#[derive(Clone)]
pub struct Repos {
    pub users: Arc<dyn UserRepo>,
    pub matches: Arc<dyn MatchRepo>,
    pub predictions: Arc<dyn PredictionRepo>,
    pub special_predictions: Arc<dyn SpecialPredictionRepo>,
    pub teams: Arc<dyn TeamRepo>,
    pub settings: Arc<dyn SettingsRepo>,
}

impl Repos {
    /// Construct a `Repos` whose impls all talk to the given Postgres pool.
    pub fn from_pool(pool: sqlx::PgPool) -> Self {
        Self {
            users: Arc::new(PgUserRepo::new(pool.clone())),
            matches: Arc::new(PgMatchRepo::new(pool.clone())),
            predictions: Arc::new(PgPredictionRepo::new(pool.clone())),
            special_predictions: Arc::new(PgSpecialPredictionRepo::new(pool.clone())),
            teams: Arc::new(PgTeamRepo::new(pool.clone())),
            settings: Arc::new(PgSettingsRepo::new(pool)),
        }
    }
}

/// Repository error. Kept narrow on purpose — handlers only need to know
/// "the persistence layer failed", not which SQL state code came back.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

pub type RepoResult<T> = Result<T, RepoError>;
