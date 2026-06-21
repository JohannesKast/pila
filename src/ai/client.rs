// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Provider-agnostic LLM client used to generate matchday recaps.
//!
//! Built on the `genai` crate so the provider is swappable via configuration.
//! The single configured `AI_API_KEY` is injected for every request through an
//! [`AuthResolver`], overriding genai's per-provider environment-variable
//! lookup, and the provider is forced via genai's `provider::model` namespace
//! syntax so the configured `AI_PROVIDER` is authoritative.

use std::time::Duration;

use chrono_tz::Tz;
use genai::chat::{ChatMessage, ChatRequest};
use genai::resolver::{AuthData, AuthResolver};
use genai::{Client, ModelIden};

/// Maximum number of generation attempts per matchday (the first try plus up to
/// four retries). On total failure nothing is stored and the next worker tick
/// tries again.
const MAX_ATTEMPTS: usize = 5;

/// Configuration for the AI recap feature. The feature is enabled only when all
/// three core values are present in the environment.
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    /// Timezone the tournament is played in, used to group matches into
    /// matchdays (a calendar day). Defaults to `America/New_York`.
    pub matchday_tz: Tz,
}

impl AiConfig {
    /// Read configuration from the environment. Returns `None` (feature off) if
    /// any of `AI_PROVIDER`, `AI_MODEL` or `AI_API_KEY` is missing or blank.
    pub fn from_env() -> Option<Self> {
        let provider = non_empty_env("AI_PROVIDER")?;
        let model = non_empty_env("AI_MODEL")?;
        let api_key = non_empty_env("AI_API_KEY")?;
        let matchday_tz = non_empty_env("AI_MATCHDAY_TZ")
            .and_then(|s| s.parse::<Tz>().ok())
            .unwrap_or(chrono_tz::America::New_York);
        Some(Self {
            provider,
            model,
            api_key,
            matchday_tz,
        })
    }

    /// `provider::model` reference understood by genai, forcing the adapter.
    pub fn model_ref(&self) -> String {
        format!("{}::{}", self.provider, self.model)
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("AI generation failed after {MAX_ATTEMPTS} attempts: {0}")]
    Failed(String),
}

/// Generate the recap text from a system + user prompt. Retries transient
/// failures up to [`MAX_ATTEMPTS`] times with a short linear backoff.
pub async fn generate(cfg: &AiConfig, system: &str, user: &str) -> Result<String, AiError> {
    let key = cfg.api_key.clone();
    let auth = AuthResolver::from_resolver_fn(
        move |_model: ModelIden| -> Result<Option<AuthData>, genai::resolver::Error> {
            Ok(Some(AuthData::from_single(key.clone())))
        },
    );
    let client = Client::builder().with_auth_resolver(auth).build();
    let model = cfg.model_ref();

    let mut last_err = String::from("unknown error");
    for attempt in 1..=MAX_ATTEMPTS {
        let req = ChatRequest::new(vec![
            ChatMessage::system(system.to_string()),
            ChatMessage::user(user.to_string()),
        ]);
        match client.exec_chat(&model, req, None).await {
            Ok(res) => match res.into_first_text() {
                Some(text) if !text.trim().is_empty() => return Ok(text),
                _ => last_err = "model returned empty response".to_string(),
            },
            Err(e) => last_err = e.to_string(),
        }
        tracing::warn!(
            attempt,
            max = MAX_ATTEMPTS,
            "AI recap generation attempt failed: {last_err}"
        );
        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
        }
    }
    Err(AiError::Failed(last_err))
}
