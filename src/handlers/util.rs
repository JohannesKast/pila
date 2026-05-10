//! Small handler helpers — cookie building, URL escaping, kickoff
//! formatting. Anything trivial enough that a fresh look reveals it.

use axum_extra::extract::cookie::{Cookie, SameSite};
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

/// Public origin for outbound links (Signal invites, magic-link mailouts).
/// Falls back to the dev default — production deployments must set
/// `BASE_URL` so users receive a clickable link.
pub fn base_url() -> String {
    std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8000".to_string())
}

pub fn build_magic_link(token: &str) -> String {
    format!("{}/play/me/{}", base_url().trim_end_matches('/'), token)
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
