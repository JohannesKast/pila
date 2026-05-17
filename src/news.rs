// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_AGE_HOURS: i64 = 72;
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug)]
pub struct NewsItem {
    pub title: String,
    pub link: String,
    pub published_iso: String,
    pub published_relative: String,
}

struct CacheInner {
    fetched_at: Instant,
    items: Vec<NewsItem>,
}

pub struct NewsCache {
    feed_url: Option<String>,
    inner: RwLock<Option<CacheInner>>,
}

impl NewsCache {
    pub fn from_env() -> Arc<Self> {
        let feed_url = std::env::var("RSS_FEED_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Arc::new(Self {
            feed_url,
            inner: RwLock::new(None),
        })
    }

    pub async fn get(&self) -> Vec<NewsItem> {
        let Some(url) = self.feed_url.as_deref() else {
            return Vec::new();
        };

        {
            let guard = self.inner.read().await;
            if let Some(cache) = guard.as_ref() {
                if cache.fetched_at.elapsed() < CACHE_TTL {
                    return cache.items.clone();
                }
            }
        }

        let mut guard = self.inner.write().await;
        if let Some(cache) = guard.as_ref() {
            if cache.fetched_at.elapsed() < CACHE_TTL {
                return cache.items.clone();
            }
        }

        match fetch_and_parse(url).await {
            Ok(items) => {
                *guard = Some(CacheInner {
                    fetched_at: Instant::now(),
                    items: items.clone(),
                });
                items
            }
            Err(e) => {
                tracing::warn!("RSS fetch failed for {}: {}", url, e);
                guard.as_ref().map(|c| c.items.clone()).unwrap_or_default()
            }
        }
    }
}

async fn fetch_and_parse(
    url: &str,
) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;
    let bytes = client
        .get(url)
        .header("User-Agent", "pila-rss/1.0")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let feed = feed_rs::parser::parse(bytes.as_ref())?;
    let cutoff = Utc::now() - chrono::Duration::hours(MAX_AGE_HOURS);

    let mut items: Vec<NewsItem> = feed
        .entries
        .into_iter()
        .filter_map(|e| {
            let published = e.published.or(e.updated)?;
            if published < cutoff {
                return None;
            }
            let title = e.title.map(|t| t.content).unwrap_or_default();
            let title = title.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let link = e
                .links
                .into_iter()
                .find(|l| !l.href.is_empty())
                .map(|l| l.href)?;
            Some(NewsItem {
                title,
                link,
                published_iso: published.to_rfc3339(),
                published_relative: relative_time(published),
            })
        })
        .collect();

    items.sort_by(|a, b| b.published_iso.cmp(&a.published_iso));
    Ok(items)
}

fn relative_time(t: DateTime<Utc>) -> String {
    let delta = Utc::now().signed_duration_since(t);
    let secs = delta.num_seconds().max(0);
    if secs < 60 {
        return "gerade".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("vor {mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("vor {hours}h");
    }
    let days = hours / 24;
    format!("vor {days}d")
}
