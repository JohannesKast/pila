//! HTTP route handlers, organised by feature area.
//!
//! Each submodule owns its routes, the form types they parse, and the
//! private Askama templates they render. Cross-cutting helpers live in
//! `util` and `services` so individual handler files stay focused on
//! request → response wiring.

pub mod admin;
pub mod auth;
pub mod index;
pub mod jersey;
pub mod leaderboard;
pub mod predictions;
pub mod services;
pub mod setup;
pub mod util;

// Re-export the actual handler functions so `main.rs` only needs to import
// `pila::handlers` to wire up its router.
pub use admin::{
    admin_create_user, admin_delete_user, admin_rename_user, admin_resend_invite,
    admin_toggle_admin,
};
pub use auth::login_magic_link;
pub use index::index;
pub use jersey::{jersey_picker_close, jersey_picker_get, jersey_post, set_language_post};
pub use leaderboard::leaderboard;
pub use predictions::{predict_match, predict_special};
pub use setup::{setup_get, setup_post};
