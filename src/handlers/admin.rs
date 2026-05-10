//! Admin user-management routes. All routes require `AdminUser` and most
//! return an HTMX partial that updates a single row in the admin table.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::Html,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AdminUser;
use crate::handlers::util::{build_magic_link, html_escape};
use crate::notifier;
use crate::repo;
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

#[derive(Deserialize)]
pub struct AdminCreateForm {
    pub name: String,
    #[serde(default)]
    pub phone_number: String,
}

pub async fn admin_create_user(
    State(state): State<AppState>,
    AdminUser(_admin): AdminUser,
    Form(form): Form<AdminCreateForm>,
) -> Result<Html<String>, (StatusCode, &'static str)> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name darf nicht leer sein."));
    }
    let phone = form.phone_number.trim();
    let phone_opt: Option<&str> = if phone.is_empty() { None } else { Some(phone) };

    let id = Uuid::new_v4();
    let token = Uuid::new_v4().to_string();

    state
        .repos
        .users
        .create(repo::user::NewUser {
            id,
            name,
            token: &token,
            is_admin: false,
            phone_number: phone_opt,
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
        magic_link: link,
        is_self: false,
    };
    Ok(render_admin_row(view, signal_enabled))
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
