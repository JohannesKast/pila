pub mod auth;
pub mod notifier;
pub mod scoring;
pub mod stage;
pub mod worker;

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
}
