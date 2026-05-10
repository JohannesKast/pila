pub mod auth;
pub mod badges;
pub mod handlers;
pub mod jersey;
pub mod news;
pub mod notifier;
pub mod repo;
pub mod scoring;
pub mod stage;
pub mod views;
pub mod worker;

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub jerseys: Arc<HashMap<String, jersey::JerseyPreset>>,
    pub news: Arc<news::NewsCache>,
    pub repos: repo::Repos,
}
