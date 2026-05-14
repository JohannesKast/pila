//! HTTP route handlers, organised by feature area.
//!
//! Each submodule owns its routes, the form types they parse, and the
//! private Askama templates they render. Cross-cutting helpers live in
//! `util` and `services` so individual handler files stay focused on
//! request → response wiring.

pub mod admin;
pub mod auth;
pub mod dev;
pub mod index;
pub mod jersey;
pub mod leaderboard;
pub mod leagues;
pub mod predictions;
pub mod services;
pub mod setup;
pub mod util;

// Re-export the actual handler functions so `main.rs` only needs to import
// `pila::handlers` to wire up its router.
pub use admin::{
    admin_create_user, admin_delete_user, admin_rename_user, admin_resend_invite,
    admin_toggle_admin, admin_users_redirect, league_users_page,
};
pub use auth::login_magic_link;
pub use dev::{
    dev_list_users, dev_panel, dev_random_results, dev_random_tips, dev_random_tips_all_users,
    dev_reset_time, dev_set_time, dev_simulate_next_matchday, dev_switch_user,
};
pub use index::index;
pub use jersey::{jersey_picker_close, jersey_picker_get, jersey_post, set_language_post};
pub use leaderboard::leaderboard;
pub use leagues::{
    league_settings_form, league_settings_save, leagues_create, leagues_list, leagues_new_form,
};
pub use predictions::{predict_match, predict_special};
pub use setup::{setup_get, setup_post};

/// Simple liveness endpoint for container orchestrators.
pub async fn healthz() -> &'static str {
    "OK"
}
