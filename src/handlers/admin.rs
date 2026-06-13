// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

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
use crate::handlers::util::{
    build_invite_link, build_magic_link, format_kickoff, html_escape, render_template, t_err,
    t_for, HandlerError,
};
use crate::notifier::{self, signal_configured};
use crate::repo;
use crate::translations::T;
use crate::views::{AdminUserView, InviteLinkView};
use crate::AppState;

#[derive(Template)]
#[template(path = "admin_row.html")]
struct AdminRowTemplate {
    u: AdminUserView,
    signal_enabled: bool,
    smtp_enabled: bool,
    t: T,
}

fn render_admin_row(u: AdminUserView, signal_enabled: bool, smtp_enabled: bool, t: T) -> Html<String> {
    let tpl = AdminRowTemplate {
        u,
        signal_enabled,
        smtp_enabled,
        t,
    };
    render_template(&tpl).unwrap_or_else(|_| Html("Internal error".to_string()))
}

/// Permission check used by every per-league admin route below: a regular
/// admin may only touch their own league, a super-admin may touch any.
fn ensure_league_access(
    state: &AppState,
    admin: &crate::auth::AuthenticatedUser,
    league_id: Uuid,
) -> Result<(), HandlerError> {
    if admin.league_id == league_id || admin.can_create_league {
        Ok(())
    } else {
        Err(t_err(
            state,
            &admin.language,
            StatusCode::FORBIDDEN,
            "error-cross-league-forbidden",
        ))
    }
}

// ─── Per-league user list page ───────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/league_users.html")]
struct LeagueUsersTemplate {
    league: repo::league::League,
    users: Vec<AdminUserView>,
    invites: Vec<InviteLinkView>,
    signal_enabled: bool,
    smtp_enabled: bool,
    is_super_admin: bool,
    t: T,
    lang_code: String,
}

#[derive(Template)]
#[template(path = "admin/invite_row.html")]
struct InviteRowTemplate {
    inv: InviteLinkView,
    t: T,
}

fn render_invite_row(inv: InviteLinkView, t: T) -> Html<String> {
    let tpl = InviteRowTemplate { inv, t };
    render_template(&tpl).unwrap_or_else(|_| Html("Internal error".to_string()))
}

fn invite_view(inv: repo::invite::InviteLink, base_url: &str) -> InviteLinkView {
    InviteLinkView {
        invite_link: build_invite_link(&inv.token, base_url),
        created_display: format_kickoff(Some(inv.created_at)),
        label: inv.label,
        id: inv.id,
    }
}

pub async fn league_users_page(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(league_id): Path<Uuid>,
) -> Result<Html<String>, HandlerError> {
    ensure_league_access(&state, &admin, league_id)?;
    let lang = &admin.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let league = state
        .repos
        .leagues
        .find_by_id(league_id)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(|| {
            t_err(
                &state,
                lang,
                StatusCode::NOT_FOUND,
                "error-league-not-found",
            )
        })?;

    let rows = state
        .repos
        .users
        .list_for_admin(league_id)
        .await
        .map_err(|_| db_err())?;

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

    let invites: Vec<InviteLinkView> = state
        .repos
        .invites
        .list_for_league(league_id)
        .await
        .map_err(|_| db_err())?
        .into_iter()
        .map(|inv| invite_view(inv, base_url))
        .collect();

    let smtp_enabled = state.smtp_config.is_some();
    let lang_code = admin.language.clone();
    let template = LeagueUsersTemplate {
        league,
        users,
        invites,
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
) -> Result<Html<String>, HandlerError> {
    ensure_league_access(&state, &admin, league_id)?;
    let lang = &admin.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let name = form.name.trim();
    if name.is_empty() {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-name-empty",
        ));
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
        .map_err(|_| db_err())?;

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
        .map_err(|_| db_err())?;

    let signal_enabled = signal_configured(&state.signal_api_url, &state.signal_from_number);
    let link = build_magic_link(&token, &state.base_url);
    let recipient_t = t_for(&state, &cfg.default_language);
    if let Some(p) = phone_opt {
        if signal_enabled {
            if let Err(e) = notifier::send_invite_via_signal(
                p,
                name,
                &link,
                &state.signal_api_url,
                &state.signal_from_number,
                &recipient_t,
            )
            .await
            {
                tracing::warn!("admin: Signal invite to {p} failed: {e}");
            }
        }
    }
    if let Some(e) = email_opt {
        if let Some(ref smtp) = state.smtp_config {
            if let Err(err) =
                crate::mail::send_invite_email(smtp, name, e, &link, &recipient_t).await
            {
                tracing::warn!("admin: email invite to {e} failed: {err}");
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
    Ok(render_admin_row(
        view,
        signal_enabled,
        state.smtp_config.is_some(),
        t_for(&state, lang),
    ))
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
) -> Result<Html<String>, HandlerError> {
    let lang = &admin.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    if id == admin.id {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-cannot-delete-self",
        ));
    }
    let target = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(|| t_err(&state, lang, StatusCode::NOT_FOUND, "error-user-not-found"))?;
    if target.league_id != admin.league_id && !admin.can_create_league {
        return Err(t_err(
            &state,
            lang,
            StatusCode::FORBIDDEN,
            "error-cross-league-forbidden",
        ));
    }
    state.repos.users.delete(id).await.map_err(|_| db_err())?;
    Ok(Html(String::new()))
}

pub async fn admin_toggle_admin(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, HandlerError> {
    let lang = &admin.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let target = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(|| t_err(&state, lang, StatusCode::NOT_FOUND, "error-user-not-found"))?;
    if target.league_id != admin.league_id && !admin.can_create_league {
        return Err(t_err(
            &state,
            lang,
            StatusCode::FORBIDDEN,
            "error-cross-league-forbidden",
        ));
    }

    let new_admin = !target.is_admin;
    if !new_admin {
        let admin_count = state
            .repos
            .users
            .count_admins()
            .await
            .map_err(|_| db_err())?;
        if admin_count <= 1 {
            return Err(t_err(
                &state,
                lang,
                StatusCode::BAD_REQUEST,
                "error-at-least-one-admin",
            ));
        }
        if id == admin.id {
            return Err(t_err(
                &state,
                lang,
                StatusCode::BAD_REQUEST,
                "error-cannot-revoke-own-admin",
            ));
        }
    }

    state
        .repos
        .users
        .set_admin(id, new_admin)
        .await
        .map_err(|_| db_err())?;

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
    Ok(render_admin_row(
        view,
        signal_configured(&state.signal_api_url, &state.signal_from_number),
        state.smtp_config.is_some(),
        t_for(&state, lang),
    ))
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
) -> Result<Html<String>, HandlerError> {
    let lang = &admin.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };
    let not_found = || t_err(&state, lang, StatusCode::NOT_FOUND, "error-user-not-found");

    let name = form.name.trim();
    if name.is_empty() {
        return Err(t_err(
            &state,
            lang,
            StatusCode::BAD_REQUEST,
            "error-name-empty",
        ));
    }
    let target_pre = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(not_found)?;
    if target_pre.league_id != admin.league_id && !admin.can_create_league {
        return Err(t_err(
            &state,
            lang,
            StatusCode::FORBIDDEN,
            "error-cross-league-forbidden",
        ));
    }
    state
        .repos
        .users
        .rename(id, name)
        .await
        .map_err(|_| db_err())?;

    let target = state
        .repos
        .users
        .find_full_by_id(id)
        .await
        .map_err(|_| db_err())?
        .ok_or_else(not_found)?;

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
    Ok(render_admin_row(
        view,
        signal_configured(&state.signal_api_url, &state.signal_from_number),
        state.smtp_config.is_some(),
        t_for(&state, lang),
    ))
}

pub async fn admin_resend_invite(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> Html<String> {
    let t = t_for(&state, &admin.language);
    let row = state.repos.users.find_full_by_id(id).await.ok().flatten();
    let Some(row) = row else {
        return Html(format!(
            r#"<span class="text-red-400 text-xs">{}</span>"#,
            html_escape(&t.get("error-user-not-found"))
        ));
    };
    let link = build_magic_link(&row.token, &state.base_url);

    let recipient_t = t_for(&state, &row.language);

    // Try Signal first if phone number exists
    if let Some(phone) = row.phone_number.as_deref() {
        if signal_configured(&state.signal_api_url, &state.signal_from_number) {
            match notifier::send_invite_via_signal(
                phone,
                &row.name,
                &link,
                &state.signal_api_url,
                &state.signal_from_number,
                &recipient_t,
            )
            .await
            {
                Ok(_) => {
                    return Html(format!(
                        r#"<span class="text-emerald-400 text-xs">{}</span>"#,
                        html_escape(&t.get("admin-resend-signal-ok"))
                    ))
                }
                Err(e) => {
                    return Html(format!(
                        r#"<span class="text-red-400 text-xs">{} {}</span>"#,
                        html_escape(&t.get("admin-resend-signal-error")),
                        html_escape(&e.to_string())
                    ))
                }
            }
        }
    }

    // Try email if address exists
    if let Some(email) = row.email.as_deref() {
        if let Some(ref smtp) = state.smtp_config {
            match crate::mail::send_invite_email(smtp, &row.name, email, &link, &recipient_t).await
            {
                Ok(_) => {
                    return Html(format!(
                        r#"<span class="text-emerald-400 text-xs">{}</span>"#,
                        html_escape(&t.get("admin-resend-email-ok"))
                    ))
                }
                Err(e) => {
                    return Html(format!(
                        r#"<span class="text-red-400 text-xs">{} {}</span>"#,
                        html_escape(&t.get("admin-resend-email-error")),
                        html_escape(&e.to_string())
                    ))
                }
            }
        }
    }

    Html(format!(
        r#"<span class="text-amber-400 text-xs">{}</span>"#,
        html_escape(&t.get("admin-resend-no-channel"))
    ))
}

// ─── Invite links ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct InviteCreateForm {
    #[serde(default)]
    pub label: String,
}

/// Generates a new shareable invite link for the league and returns the
/// rendered row so HTMX can prepend it to the invite list.
pub async fn admin_create_invite(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(league_id): Path<Uuid>,
    Form(form): Form<InviteCreateForm>,
) -> Result<Html<String>, HandlerError> {
    ensure_league_access(&state, &admin, league_id)?;
    let lang = &admin.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    let label = form.label.trim();
    let label_opt: Option<&str> = if label.is_empty() { None } else { Some(label) };
    let token = Uuid::new_v4().to_string();

    let id = state
        .repos
        .invites
        .create(league_id, &token, label_opt)
        .await
        .map_err(|_| db_err())?;

    let inv = repo::invite::InviteLink {
        id,
        league_id,
        token,
        label: label_opt.map(|s| s.to_string()),
        created_at: chrono::Utc::now(),
    };
    Ok(render_invite_row(
        invite_view(inv, &state.base_url),
        t_for(&state, &admin.language),
    ))
}

/// Revokes (deletes) an invite link. The token stops working immediately.
/// Returns an empty `200` body so HTMX swaps out (removes) the row.
///
/// Idempotent: revoking an id that no longer exists still returns the empty
/// `200` so the HTMX `outerHTML` swap removes the row. Returning a 404 here
/// would make HTMX skip the swap and leave a dead row on screen.
pub async fn admin_revoke_invite(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, HandlerError> {
    let lang = &admin.language;
    let db_err = || {
        t_err(
            &state,
            lang,
            StatusCode::INTERNAL_SERVER_ERROR,
            "error-database",
        )
    };

    // Already gone (e.g. a double-click) — remove the row idempotently.
    let Some(invite) = state
        .repos
        .invites
        .find_by_id(id)
        .await
        .map_err(|_| db_err())?
    else {
        return Ok(Html(String::new()));
    };
    ensure_league_access(&state, &admin, invite.league_id)?;

    state.repos.invites.delete(id).await.map_err(|_| db_err())?;
    Ok(Html(String::new()))
}
