// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

//! Provider-agnostic LLM client used to generate matchday recaps.
//!
//! This intentionally avoids a heavy provider SDK: the feature only needs one
//! chat-completion style call, so we build the small Gemini/OpenAI-compatible
//! request shapes directly and keep the provider selected by configuration.

use std::time::Duration;

use chrono_tz::Tz;
use reqwest::Client;
use serde_json::{json, Value};

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
    /// Optional endpoint base for OpenAI-compatible providers not known by
    /// name. When set, this overrides the built-in base URL for all
    /// OpenAI-compatible providers.
    pub base_url: Option<String>,
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
        let base_url = non_empty_env("AI_BASE_URL");
        let matchday_tz = non_empty_env("AI_MATCHDAY_TZ")
            .and_then(|s| s.parse::<Tz>().ok())
            .unwrap_or(chrono_tz::America::New_York);
        Some(Self {
            provider,
            model,
            api_key,
            base_url,
            matchday_tz,
        })
    }

    /// Provider/model reference stored with generated reports for debugging.
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
    #[error("unsupported AI provider `{0}`")]
    UnsupportedProvider(String),
    #[error("AI provider `{0}` requires AI_BASE_URL")]
    MissingBaseUrl(String),
    #[error("AI request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("AI request failed with status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("AI response did not contain text")]
    MissingText,
    #[error("AI generation failed after {MAX_ATTEMPTS} attempts: {0}")]
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    Gemini,
    OpenAiCompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthKind {
    Bearer,
    GoogleApiKey,
}

#[derive(Debug, Clone)]
struct RequestSpec {
    url: String,
    auth: AuthKind,
    provider: ProviderKind,
    body: Value,
}

/// Generate the recap text from a system + user prompt. Retries transient
/// failures up to [`MAX_ATTEMPTS`] times with a short linear backoff.
pub async fn generate(cfg: &AiConfig, system: &str, user: &str) -> Result<String, AiError> {
    let spec = build_request_spec(cfg, system, user)?;
    let client = Client::new();

    let mut last_err = String::from("unknown error");
    for attempt in 1..=MAX_ATTEMPTS {
        match send_once(&client, cfg, &spec).await {
            Ok(text) => return Ok(text),
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

fn build_request_spec(cfg: &AiConfig, system: &str, user: &str) -> Result<RequestSpec, AiError> {
    let provider = cfg.provider.trim().to_ascii_lowercase();
    match provider.as_str() {
        "gemini" | "google" | "google-ai" => Ok(RequestSpec {
            url: format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                cfg.model
            ),
            auth: AuthKind::GoogleApiKey,
            provider: ProviderKind::Gemini,
            body: json!({
                "systemInstruction": {
                    "parts": [{ "text": system }]
                },
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": user }]
                }]
            }),
        }),
        _ => {
            let base = openai_compatible_base(&provider, cfg.base_url.as_deref())?;
            Ok(RequestSpec {
                url: format!("{}/chat/completions", base.trim_end_matches('/')),
                auth: AuthKind::Bearer,
                provider: ProviderKind::OpenAiCompatible,
                body: json!({
                    "model": cfg.model,
                    "messages": [
                        { "role": "system", "content": system },
                        { "role": "user", "content": user }
                    ]
                }),
            })
        }
    }
}

fn openai_compatible_base(
    provider: &str,
    configured_base: Option<&str>,
) -> Result<String, AiError> {
    if let Some(base) = configured_base {
        return Ok(base.to_string());
    }

    match provider {
        "openai" => Ok("https://api.openai.com/v1".to_string()),
        "groq" => Ok("https://api.groq.com/openai/v1".to_string()),
        "xai" | "x-ai" => Ok("https://api.x.ai/v1".to_string()),
        "deepseek" => Ok("https://api.deepseek.com".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1".to_string()),
        "ollama" => Ok("http://localhost:11434/v1".to_string()),
        "openai-compatible" | "compatible" => Err(AiError::MissingBaseUrl(provider.to_string())),
        other => Err(AiError::UnsupportedProvider(other.to_string())),
    }
}

async fn send_once(client: &Client, cfg: &AiConfig, spec: &RequestSpec) -> Result<String, AiError> {
    let req = match spec.auth {
        AuthKind::Bearer => client.post(&spec.url).bearer_auth(&cfg.api_key),
        AuthKind::GoogleApiKey => client
            .post(&spec.url)
            .header("x-goog-api-key", &cfg.api_key),
    };
    let res = req.json(&spec.body).send().await?;
    let status = res.status();
    let body = res.text().await?;
    if !status.is_success() {
        return Err(AiError::HttpStatus {
            status: status.as_u16(),
            body: excerpt(&body),
        });
    }
    let json: Value = serde_json::from_str(&body).map_err(|_| AiError::MissingText)?;
    parse_response_text(spec.provider, &json)
}

fn parse_response_text(provider: ProviderKind, body: &Value) -> Result<String, AiError> {
    let text = match provider {
        ProviderKind::Gemini => body
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>()
            }),
        ProviderKind::OpenAiCompatible => body
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());

    text.ok_or(AiError::MissingText)
}

fn excerpt(body: &str) -> String {
    const MAX_CHARS: usize = 500;
    let mut out: String = body.chars().take(MAX_CHARS).collect();
    if body.chars().nth(MAX_CHARS).is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(provider: &str) -> AiConfig {
        AiConfig {
            provider: provider.to_string(),
            model: "test-model".to_string(),
            api_key: "secret-key".to_string(),
            base_url: None,
            matchday_tz: chrono_tz::America::New_York,
        }
    }

    #[test]
    fn gemini_request_uses_header_auth_and_keeps_key_out_of_url() {
        let spec = build_request_spec(&cfg("gemini"), "system", "user").unwrap();
        assert_eq!(spec.provider, ProviderKind::Gemini);
        assert_eq!(spec.auth, AuthKind::GoogleApiKey);
        assert!(!spec.url.contains("secret-key"));
        assert_eq!(spec.body["systemInstruction"]["parts"][0]["text"], "system");
        assert_eq!(spec.body["contents"][0]["parts"][0]["text"], "user");
    }

    #[test]
    fn openai_compatible_request_uses_provider_endpoint() {
        let spec = build_request_spec(&cfg("groq"), "system", "user").unwrap();
        assert_eq!(spec.provider, ProviderKind::OpenAiCompatible);
        assert_eq!(spec.auth, AuthKind::Bearer);
        assert_eq!(spec.url, "https://api.groq.com/openai/v1/chat/completions");
        assert_eq!(spec.body["model"], "test-model");
        assert_eq!(spec.body["messages"][0]["role"], "system");
        assert_eq!(spec.body["messages"][1]["content"], "user");
    }

    #[test]
    fn configured_base_url_overrides_openai_compatible_endpoint() {
        let mut cfg = cfg("openai-compatible");
        cfg.base_url = Some("https://llm.example.test/v1/".to_string());
        let spec = build_request_spec(&cfg, "system", "user").unwrap();
        assert_eq!(spec.url, "https://llm.example.test/v1/chat/completions");
    }

    #[test]
    fn unsupported_provider_is_rejected_before_http_request() {
        let err = build_request_spec(&cfg("anthropic"), "system", "user").unwrap_err();
        assert!(matches!(err, AiError::UnsupportedProvider(_)));
    }

    #[test]
    fn parses_openai_compatible_text() {
        let body = json!({
            "choices": [{
                "message": { "content": "  recap text  " }
            }]
        });
        let text = parse_response_text(ProviderKind::OpenAiCompatible, &body).unwrap();
        assert_eq!(text, "recap text");
    }

    #[test]
    fn parses_gemini_text_parts() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "recap " },
                        { "text": "text" }
                    ]
                }
            }]
        });
        let text = parse_response_text(ProviderKind::Gemini, &body).unwrap();
        assert_eq!(text, "recap text");
    }

    #[test]
    fn empty_provider_response_is_rejected() {
        let body = json!({ "choices": [{ "message": { "content": "   " } }] });
        let err = parse_response_text(ProviderKind::OpenAiCompatible, &body).unwrap_err();
        assert!(matches!(err, AiError::MissingText));
    }
}
