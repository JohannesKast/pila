//! Small handler helpers — cookie building, URL escaping, kickoff
//! formatting. Anything trivial enough that a fresh look reveals it.

use axum_extra::extract::cookie::{Cookie, SameSite};

use askama::Template;
use axum::{
    http::{HeaderMap, StatusCode},
    response::Html,
};
/// Construct the login cookie that pins a magic-link token to the browser.
/// Centralised so cookie attributes stay consistent across login + setup.
pub fn make_login_cookie(token: String) -> Cookie<'static> {
    // 1 year in seconds; persistent so the admin survives browser restarts
    Cookie::build(("pila_token", token))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(365))
        .build()
}

/// Non-HttpOnly sibling of the login cookie, readable by JS so HTMX can
/// send it as a `X-CSRF-Token` header. Part of a double-submit-cookie
/// CSRF defence that works with `SameSite=Lax`.
pub fn make_csrf_cookie(token: String) -> Cookie<'static> {
    Cookie::build(("pila_csrf", token))
        .path("/")
        .http_only(false)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(365))
        .build()
}

/// Public origin for outbound links (Signal invites, magic-link mailouts).
/// Reads from `AppState` — cached at startup, never calls `env::var` at
/// request time.
pub fn base_url(state: &crate::AppState) -> &str {
    &state.base_url
}

pub fn build_magic_link(token: &str, base: &str) -> String {
    format!("{}/play/me/{}", base.trim_end_matches('/'), token)
}

/// Minimal HTML escaper for inline error fragments. We never trust an
/// upstream `tower_http` impl here because the inline-fragment paths bypass
/// Askama's auto-escaping.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Map an ISO-3166 alpha-2 code to a flagcdn URL. Empty/`None` codes return
/// an empty string so templates can `{% if flag %}` without panicking.
pub fn flag_url(code: &Option<String>) -> String {
    match code {
        Some(c) if !c.is_empty() => format!("https://flagcdn.com/w40/{c}.png"),
        _ => String::new(),
    }
}

pub fn format_kickoff(dt: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match dt {
        Some(d) => d
            .with_timezone(&chrono_tz::Europe::Berlin)
            .format("%d.%m.%Y %H:%M")
            .to_string(),
        None => "TBD".to_string(),
    }
}

/// Centralised translation lookup. Falls back to `de` in production; tests
/// that build an `AppState` without translations get an empty bundle so
/// `.get(key)` returns the key itself — never panics.
pub fn t_for(state: &crate::AppState, lang: &str) -> crate::translations::T {
    state
        .translations
        .get(lang)
        .or_else(|| state.translations.get("de"))
        .cloned()
        .unwrap_or_default()
}

/// Render an Askama template into an HTML response, mapping render errors
/// to a 500 with a tracing log instead of panicking via `.unwrap()`.
///
/// The error body is intentionally English-only: template render failures
/// are programmer bugs that the operator inspects via logs; the response
/// body just signals the failure mode to the browser.
pub fn render_template<T: Template>(template: &T) -> Result<Html<String>, HandlerError> {
    template.render().map(Html).map_err(|e| {
        tracing::error!(%e, "template error");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal error".to_string(),
        )
    })
}

/// Translated error response. Use [`HandlerError::new`] to build one and rely
/// on the `IntoResponse` impl (via Axum's tuple-error conversion) to return
/// it from handlers.
pub type HandlerError = (StatusCode, String);

/// Build a translated handler error: looks up `key` in the caller-specified
/// language, falling back to `de`. The status is what the client sees; the
/// body is the translated message.
pub fn t_err(state: &crate::AppState, lang: &str, status: StatusCode, key: &str) -> HandlerError {
    (status, t_for(state, lang).get(key))
}

/// Pre-auth flavour of [`t_err`]: extracts the preferred locale from the
/// request's `Accept-Language` header. Falls back to `de` when the header is
/// missing or matches no supported locale.
pub fn t_err_from_headers(
    state: &crate::AppState,
    headers: &HeaderMap,
    status: StatusCode,
    key: &str,
) -> HandlerError {
    let lang = preferred_lang(headers);
    t_err(state, &lang, status, key)
}

const SUPPORTED_LOCALES: &[&str] = &["de", "en", "es", "fr"];

/// Parse `Accept-Language`, return the first supported locale or `de`.
/// Token quality scores are ignored — the header is read left-to-right.
pub fn preferred_lang(headers: &HeaderMap) -> String {
    let Some(value) = headers.get(axum::http::header::ACCEPT_LANGUAGE) else {
        return "de".into();
    };
    let Ok(s) = value.to_str() else {
        return "de".into();
    };
    for raw in s.split(',') {
        let tag = raw.split(';').next().unwrap_or("").trim();
        let primary = tag.split('-').next().unwrap_or("").to_ascii_lowercase();
        if SUPPORTED_LOCALES.contains(&primary.as_str()) {
            return primary;
        }
    }
    "de".into()
}
