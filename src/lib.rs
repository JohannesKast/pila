pub mod auth;
pub mod badges;
pub mod handlers;
pub mod jersey;
pub mod news;
pub mod notifier;
pub mod repo;
pub mod scoreboard;
pub mod scoring;
pub mod stage;
pub mod translations;
pub mod views;
pub mod worker;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Maximum in-flight requests the server handles concurrently.
/// Requests beyond this limit queue up via the semaphore middleware,
/// providing backpressure without dropping connections.
pub const MAX_CONCURRENT_REQUESTS: usize = 30;

#[derive(Clone)]
pub struct AppState {
    pub jerseys: Arc<HashMap<String, jersey::JerseyPreset>>,
    pub news: Arc<news::NewsCache>,
    pub repos: repo::Repos,
    pub translations: HashMap<String, translations::T>,
    /// Global concurrency semaphore — one permit per in-flight request.
    /// The middleware layer acquires a permit before handing the request
    /// to the router and releases it when the response is sent.
    pub concurrency_limit: Arc<Semaphore>,
}
