use fluent_bundle::{FluentBundle, FluentResource};
use std::collections::HashMap;
use std::sync::Arc;
use unic_langid::LanguageIdentifier;

/// Cheap-to-clone pre-formatted string table for one locale.
///
/// FTL files live in `locales/`. All messages are extracted at startup;
/// `T::get` returns the translated string or the key itself as fallback so
/// a missing translation is visible but never panics at runtime.
#[derive(Clone)]
pub struct T(Arc<HashMap<String, String>>);

impl T {
    pub fn get(&self, key: &str) -> String {
        self.0.get(key).cloned().unwrap_or_else(|| key.to_string())
    }
}

/// Parse FTL content and collect all top-level message IDs.
/// Only lines that start with an identifier character (not `#`, `-`, `.`,
/// whitespace, or empty) and contain `=` are treated as message lines.
fn extract_ids(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let first = line.chars().next()?;
            if first == '#' || first == '-' || first == '.' || first.is_whitespace() {
                return None;
            }
            let (key, _) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some(key.to_string())
        })
        .collect()
}

fn load_one(code: &str) -> T {
    let path = format!("locales/{code}.ftl");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Cannot read translation file {path}: {e}"));

    let ids = extract_ids(&content);

    let lang_id: LanguageIdentifier = code
        .parse()
        .unwrap_or_else(|e| panic!("Invalid locale code '{code}': {e}"));

    let mut bundle = FluentBundle::new(vec![lang_id]);
    let resource = FluentResource::try_new(content).unwrap_or_else(|(r, _)| r);
    let _ = bundle.add_resource(resource);

    let mut strings = HashMap::with_capacity(ids.len());
    for id in ids {
        if let Some(msg) = bundle.get_message(&id) {
            if let Some(pattern) = msg.value() {
                let mut errors = vec![];
                let value = bundle.format_pattern(pattern, None, &mut errors);
                strings.insert(id, value.to_string());
            }
        }
    }

    T(Arc::new(strings))
}

/// Load all supported locales from `locales/*.ftl` at startup.
/// Panics if a file is missing or unparseable.
pub fn load_all() -> HashMap<String, T> {
    ["de", "en", "es", "fr"]
        .iter()
        .map(|&code| (code.to_string(), load_one(code)))
        .collect()
}
