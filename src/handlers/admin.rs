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
use crate::handlers::util::{build_magic_link, html_escape};
use crate::notifier;
use crate::repo;
use crate::translations::T;
use crate::views::AdminUserView;
use crate::AppState;

#[derive(Template)]
#[template(path = "admin_row.html")]
struct AdminRowTemplate {
    u: AdminUserView,
    signal_enabled: bool,
}

fn render_admin_row(u: AdminUserView, signal_enabled: bool) -> Html<String> {
    let tpl = AdminRowTemplate { u, signal_enabled };
    Html(tpl.render().unwrap())
}

fn t_for(state: &AppState, lang: &str) -> T {
    state
        .translations
        .get(lang)
        .or_else(|| state.translations.get("de"))
        .expect("de locale always present")
        .clone()
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

    let users: Vec<AdminUserView> = rows
        .into_iter()
        .map(|r| AdminUserView {
            magic_link: build_magic_link(&r.token),
            is_self: r.id == admin.id,
            id: r.id,
            name: r.name,
            phone_number: r.phone_number,
            is_admin: r.is_admin,
            can_create_league: r.can_create_league,
        })
        .collect();

    let lang_code = admin.language.clone();
    let template = LeagueUsersTemplate {
        league,
        users,
        signal_enabled: notifier::signal_configured(),
        is_super_admin: admin.can_create_league,
        t: t_for(&state, &admin.language),
        lang_code,
    };
    Ok(Html(template.render().unwrap()))
}

#[derive(Deserialize)]
pub struct AdminCreateForm {
    pub name: String,
    #[serde(default)]
    pub phone_number: String,
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
            league_id,
            language: &cfg.default_language,
        })
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?;

    let signal_enabled = notifier::signal_configured();
    let link = build_magic_link(&token);
    if let Some(p) = phone_opt {
        if signal_enabled {
            if let Err(e) = notifier::send_invite_via_signal(p, name, &link).await {
                tracing::warn!("Admin: Signal-Einladung an {p} fehlgeschlagen: {e}");
            }
        }
    }

    let view = AdminUserView {
        id,
        name: name.to_string(),
        phone_number: phone_opt.map(|s| s.to_string()),
        is_admin: false,
        can_create_league: false,
        magic_link: link,
        is_self: false,
    };
    Ok(render_admin_row(view, signal_enabled))
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
        magic_link: build_magic_link(&target.token),
        is_self: id == admin.id,
        id,
        name: target.name,
        phone_number: target.phone_number,
        is_admin: new_admin,
        can_create_league: target.can_create_league,
    };
    Ok(render_admin_row(view, notifier::signal_configured()))
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
        magic_link: build_magic_link(&target.token),
        is_self: id == admin.id,
        id,
        name: target.name,
        phone_number: target.phone_number,
        is_admin: target.is_admin,
        can_create_league: target.can_create_league,
    };
    Ok(render_admin_row(view, notifier::signal_configured()))
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
    let Some(phone) = row.phone_number.as_deref() else {
        return Html(
            r#"<span class="text-amber-400 text-xs">Keine Telefonnummer hinterlegt</span>"#
                .to_string(),
        );
    };
    if !notifier::signal_configured() {
        return Html(
            r#"<span class="text-amber-400 text-xs">Signal nicht konfiguriert</span>"#
                .to_string(),
        );
    }
    let link = build_magic_link(&row.token);
    match notifier::send_invite_via_signal(phone, &row.name, &link).await {
        Ok(_) => Html(
            r#"<span class="text-emerald-400 text-xs">✓ gesendet</span>"#.to_string(),
        ),
        Err(e) => Html(format!(
            r#"<span class="text-red-400 text-xs">✗ Fehler: {}</span>"#,
            html_escape(&e.to_string())
        )),
    }
}
