use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

/// Jersey variant — used for grouping in the picker UI.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JerseyVariant {
    Home,
    Away,
    Fan,
}

impl Default for JerseyVariant {
    fn default() -> Self {
        Self::Home
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JerseyPreset {
    pub body: String,
    pub accent: String,
    pub pattern: String,
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub variant: JerseyVariant,
}

#[derive(Debug, Deserialize)]
struct JerseysFile {
    presets: HashMap<String, JerseyPreset>,
}

const JERSEYS_JSON: &str = include_str!("../handoff/jerseys.json");

pub fn load() -> Arc<HashMap<String, JerseyPreset>> {
    let parsed: JerseysFile =
        serde_json::from_str(JERSEYS_JSON).expect("handoff/jerseys.json must be valid");
    Arc::new(parsed.presets)
}

pub fn get<'a>(
    presets: &'a HashMap<String, JerseyPreset>,
    key: &str,
) -> &'a JerseyPreset {
    presets
        .get(key)
        .or_else(|| presets.get("classic"))
        .expect("'classic' jersey preset must exist as fallback")
}
