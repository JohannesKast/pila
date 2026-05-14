pub mod auth;
pub mod badges;
pub mod handlers;
pub mod jersey;
pub mod mail;
pub mod news;
pub mod notifier;
pub mod repo;
pub mod scoreboard;
pub mod scoring;
pub mod stage;
pub mod time;
pub mod translations;
pub mod views;
pub mod worker;

use std::collections::HashMap;
use std::sync::Arc;
use sqlx::PgPool;
use tokio::sync::Semaphore;

/// Maximum in-flight requests the server handles concurrently.
/// Requests beyond this limit queue up via the semaphore middleware,
/// providing backpressure without dropping connections.
pub const MAX_CONCURRENT_REQUESTS: usize = 30;

#[derive(Clone)]
pub struct AppState {
    /// Direct pool handle used by `setup_post` for its multi-table
    /// transaction. `None` in tests that use in-memory repos.
    pub db: Option<PgPool>,
    pub jerseys: Arc<HashMap<String, jersey::JerseyPreset>>,
    pub news: Arc<news::NewsCache>,
    pub repos: repo::Repos,
    pub translations: HashMap<String, translations::T>,
    /// Global concurrency semaphore — one permit per in-flight request.
    /// The middleware layer acquires a permit before handing the request
    /// to the router and releases it when the response is sent.
    pub concurrency_limit: Arc<Semaphore>,
    /// Cached at startup so handlers and the worker don't call
    /// `std::env::var` on every request / notification tick.
    pub base_url: String,
    pub signal_api_url: Option<String>,
    pub signal_from_number: Option<String>,
    pub signal_group_id: Option<String>,
    /// Reusable HTTP client (connection pooling, keep-alive).  Built once
    /// at startup so `notifier` and `news` don't create a fresh client on
    /// every request / notification tick.
    pub http_client: reqwest::Client,
    /// Mock time for dev/testing. When `Some(t)`, `time::now()` returns `t`
    /// instead of `Utc::now()`. Always `None` in production.
    pub mock_now: time::MockTime,
    /// Whether dev mode is enabled (`PILA_DEV_MODE=true`).
    pub dev_mode: bool,
    /// Global SMTP configuration. `None` = email delivery disabled.
    pub smtp_config: Option<crate::mail::SmtpConfig>,
}
