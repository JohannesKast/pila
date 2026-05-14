//! Admin user-management routes. All routes require `AdminUser` and most
//! return an HTMX partial that updates a single row in the admin table.
//!
//! User management is *always* scoped to a league. Non-super-admins may only
//! act on users in their own league; the super-admin (`can_create_league`)
//! may operate on any league via the `/admin/leagues/:id/users` URLs.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, Redirect},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AdminUser;
use crate::handlers::util::{build_magic_link, html_escape, render_template, t_for};
use crate::notifier::{self, signal_configured};
use crate::repo;
use crate::translations::T;
use crate::views::AdminUserView;
use crate::AppState;

#[derive(Template)]
#[template(path = "admin_row.html")]
struct AdminRowTemplate {
    u: AdminUserView,
    signal_enabled: bool,
    smtp_enabled: bool,
}

fn render_admin_row(u: AdminUserView, signal_enabled: bool, smtp_enabled: bool) -> Html<String> {
    let tpl = AdminRowTemplate { u, signal_enabled, smtp_enabled };
    render_template(&tpl).unwrap_or_else(|_| Html("Interner Fehler".to_string()))
}

/// Permission check used by every per-league admin route below: a regular
/// admin may only touch their own league, a super-admin may touch any.
fn ensure_league_access(
    admin: &crate::auth::AuthenticatedUser,
    league_id: Uuid,
) -> Result<(), (StatusCode, &'static str)> {
    if admin.league_id == league_id || admin.can_create_league {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "Cross-League-Aktionen erfordern can_create_league.",
        ))
    }
}

// ─── Per-league user list page ───────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/league_users.html")]
struct LeagueUsersTemplate {
    league: repo::league::League,
    users: Vec<AdminUserView>,
    signal_enabled: bool,
    smtp_enabled: bool,
    is_super_admin: bool,
    t: T,
    lang_code: String,
}

pub async fn league_users_page(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(league_id): Path<Uuid>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    ensure_league_access(&admin, league_id)?;

    let league = state
        .repos
        .leagues
        .find_by_id(league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::NOT_FOUND, "Liga nicht gefunden."))?;

    let rows = state
        .repos
        .users
        .list_for_admin(league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let base_url = &state.base_url;
    let sig_cfg = (&state.signal_api_url, &state.signal_from_number);

    let users: Vec<AdminUserView> = rows
        .into_iter()
        .map(|r| AdminUserView {
            magic_link: build_magic_link(&r.token, base_url),
            is_self: r.id == admin.id,
            id: r.id,
            name: r.name,
            phone_number: r.phone_number,
            email: r.email,
            is_admin: r.is_admin,
            can_create_league: r.can_create_league,
        })
        .collect();

    let smtp_enabled = state.smtp_config.is_some();
    let lang_code = admin.language.clone();
    let template = LeagueUsersTemplate {
        league,
        users,
        signal_enabled: signal_configured(sig_cfg.0, sig_cfg.1),
        smtp_enabled,
        is_super_admin: admin.can_create_league,
        t: t_for(&state, &admin.language),
        lang_code,
    };
    render_template(&template)
}

#[derive(Deserialize)]
pub struct AdminCreateForm {
    pub name: String,
    #[serde(default)]
    pub phone_number: String,
    #[serde(default)]
    pub email: String,
}

pub async fn admin_create_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(league_id): Path<Uuid>,
    Form(form): Form<AdminCreateForm>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    ensure_league_access(&admin, league_id)?;

    let name = form.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name darf nicht leer sein."));
    }
    let phone = form.phone_number.trim();
    let phone_opt: Option<&str> = if phone.is_empty() { None } else { Some(phone) };
    let email = form.email.trim();
    let email_opt: Option<&str> = if email.is_empty() { None } else { Some(email) };

    let id = Uuid::new_v4();
    let token = Uuid::new_v4().to_string();

    let cfg = state
        .repos
        .leagues
        .get_config(league_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    state
        .repos
        .users
        .create(repo::user::NewUser {
            id,
            name,
            token: &token,
            is_admin: false,
            phone_number: phone_opt,
            email: email_opt,
            league_id,
            language: &cfg.default_language,
        })
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let signal_enabled = signal_configured(&state.signal_api_url, &state.signal_from_number);
    let link = build_magic_link(&token, &state.base_url);
    if let Some(p) = phone_opt {
        if signal_enabled {
            if let Err(e) = notifier::send_invite_via_signal(p, name, &link, &state.signal_api_url, &state.signal_from_number).await {
                tracing::warn!("Admin: Signal-Einladung an {p} fehlgeschlagen: {e}");
            }
        }
    }
    if let Some(e) = email_opt {
        if let Some(ref smtp) = state.smtp_config {
            if let Err(err) = crate::mail::send_invite_email(smtp, name, e, &link).await {
                tracing::warn!("Admin: E-Mail-Einladung an {e} fehlgeschlagen: {err}");
            }
        }
    }

    let view = AdminUserView {
        id,
        name: name.to_string(),
        phone_number: phone_opt.map(|s| s.to_string()),
        email: email_opt.map(|s| s.to_string()),
        is_admin: false,
        can_create_league: false,
        magic_link: link,
        is_self: false,
    };
    Ok(render_admin_row(view, signal_enabled, state.smtp_config.is_some()))
}

/// Convenience redirect: `/admin/users` is the index "User-Verwaltung" entry
/// point — sends the admin straight to their own league's user list. Kept as
/// a stable endpoint so external bookmarks / scripts don't break.
pub async fn admin_users_redirect(AdminUser(admin): AdminUser) -> Redirect {
    Redirect::to(&format!("/admin/leagues/{}/users", admin.league_id))
}

pub async fn admin_delete_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    if id == admin.id {
        return Err((
            StatusCode::BAD_REQUEST,
            "Du kannst dich nicht selbst löschen.",
        ));
    }
    let target = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::NOT_FOUND, "User nicht gefunden."))?;
    if target.league_id != admin.league_id && !admin.can_create_league {
        return Err((
            StatusCode::FORBIDDEN,
            "Cross-League-Aktionen erfordern can_create_league.",
        ));
    }
    state
        .repos
        .users
        .delete(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
    Ok(Html(String::new()))
}

pub async fn admin_toggle_admin(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    let target = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::NOT_FOUND, "User nicht gefunden."))?;
    if target.league_id != admin.league_id && !admin.can_create_league {
        return Err((
            StatusCode::FORBIDDEN,
            "Cross-League-Aktionen erfordern can_create_league.",
        ));
    }

    let new_admin = !target.is_admin;
    if !new_admin {
        let admin_count = state
            .repos
            .users
            .count_admins()
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;
        if admin_count <= 1 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Mindestens ein Admin muss bestehen bleiben.",
            ));
        }
        if id == admin.id {
            return Err((
                StatusCode::BAD_REQUEST,
                "Du kannst dir nicht selbst die Adminrechte entziehen.",
            ));
        }
    }

    state
        .repos
        .users
        .set_admin(id, new_admin)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let view = AdminUserView {
        magic_link: build_magic_link(&target.token, &state.base_url),
        is_self: id == admin.id,
        id,
        name: target.name,
        phone_number: target.phone_number,
        email: target.email,
        is_admin: new_admin,
        can_create_league: target.can_create_league,
    };
    Ok(render_admin_row(view, signal_configured(&state.signal_api_url, &state.signal_from_number), state.smtp_config.is_some()))
}

#[derive(Deserialize)]
pub struct AdminRenameForm {
    pub name: String,
}

pub async fn admin_rename_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
    Form(form): Form<AdminRenameForm>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name darf nicht leer sein."));
    }
    let target_pre = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::NOT_FOUND, "User nicht gefunden."))?;
    if target_pre.league_id != admin.league_id && !admin.can_create_league {
        return Err((
            StatusCode::FORBIDDEN,
            "Cross-League-Aktionen erfordern can_create_league.",
        ));
    }
    state
        .repos
        .users
        .rename(id, name)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let target = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::NOT_FOUND, "User nicht gefunden."))?;

    let view = AdminUserView {
        magic_link: build_magic_link(&target.token, &state.base_url),
        is_self: id == admin.id,
        id,
        name: target.name,
        phone_number: target.phone_number,
        email: target.email,
        is_admin: target.is_admin,
        can_create_league: target.can_create_league,
    };
    Ok(render_admin_row(view, signal_configured(&state.signal_api_url, &state.signal_from_number), state.smtp_config.is_some()))
}

pub async fn admin_resend_invite(
    State(state): State<AppState>,
    AdminUser(_admin): AdminUser,
    Path(id): Path<Uuid>,
) -> Html<String> {
    let row = state.repos.users.find_full_by_id(id).await.ok().flatten();
    let Some(row) = row else {
        return Html(
            r#"<span class="text-red-400 text-xs">User nicht gefunden</span>"#.to_string(),
        );
    };
    let link = build_magic_link(&row.token, &state.base_url);

    // Try Signal first if phone number exists
    if let Some(phone) = row.phone_number.as_deref() {
        if signal_configured(&state.signal_api_url, &state.signal_from_number) {
            match notifier::send_invite_via_signal(phone, &row.name, &link, &state.signal_api_url, &state.signal_from_number).await {
                Ok(_) => return Html(
                    r#"<span class="text-emerald-400 text-xs">✓ Signal gesendet</span>"#.to_string(),
                ),
                Err(e) => return Html(format!(
                    r#"<span class="text-red-400 text-xs">✗ Signal-Fehler: {}</span>"#,
                    html_escape(&e.to_string())
                )),
            }
        }
    }

    // Try email if address exists
    if let Some(email) = row.email.as_deref() {
        if let Some(ref smtp) = state.smtp_config {
            match crate::mail::send_invite_email(smtp, &row.name, email, &link).await {
                Ok(_) => return Html(
                    r#"<span class="text-emerald-400 text-xs">✓ E-Mail gesendet</span>"#.to_string(),
                ),
                Err(e) => return Html(format!(
                    r#"<span class="text-red-400 text-xs">✗ E-Mail-Fehler: {}</span>"#,
                    html_escape(&e.to_string())
                )),
            }
        }
    }

    Html(
        r#"<span class="text-amber-400 text-xs">Keine Kontaktdaten oder Versandkanal konfiguriert</span>"#.to_string(),
    )
}
