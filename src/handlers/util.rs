//! Small handler helpers — cookie building, URL escaping, kickoff
//! formatting. Anything trivial enough that a fresh look reveals it.

use axum_extra::extract::cookie::{Cookie, SameSite};

use askama::Template;
use axum::{http::StatusCode, response::Html};
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

/// Centralised translation lookup. Falls back to `de` — the only locale
/// guaranteed to be loaded at startup — so callers can safely `.clone()`
/// without panicking.
pub fn t_for(state: &crate::AppState, lang: &str) -> crate::translations::T {
    state
        .translations
        .get(lang)
        .or_else(|| state.translations.get("de"))
        .expect("de locale always present")
        .clone()
}

/// Render an Askama template into an HTML response, mapping render errors
/// to a 500 with a tracing log instead of panicking via `.unwrap()`.
pub fn render_template<T: Template>(
    template: &T,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    template.render().map(Html).map_err(|e| {
        tracing::error!(%e, "template error");
        (StatusCode::INTERNAL_SERVER_ERROR, "Interner Fehler")
    })
}
