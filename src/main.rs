use axum::{routing::get, Router};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use std::sync::Arc;

use pila::handlers;
use pila::repo::Repos;
use pila::scoreboard::EspnClient;
use pila::{news, worker, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Pila Application...");

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    tracing::info!("Running database migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run DB migrations");

    let repos = Repos::from_pool(pool);
    let state = AppState {
        jerseys: pila::jersey::load(),
        news: news::NewsCache::from_env(),
        repos: repos.clone(),
        translations: pila::translations::load_all(),
    };

    if let Err(e) = worker::bootstrap_notifications(&repos).await {
        tracing::warn!("Notification bootstrap failed: {:?}", e);
    }

    let scoreboard: Arc<dyn pila::scoreboard::ScoreboardClient> = Arc::new(EspnClient::new());
    worker::start_background_worker(repos, scoreboard).await;

    let app = build_router().with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".into());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().expect("Invalid PORT");
    tracing::info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

/// Wire up every route. Pulled out of `main` so integration tests can spin
/// up a real `axum::Router` against a fake `AppState` (see `tests/`).
fn build_router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::index))
        .route("/play/me/:token", get(handlers::login_magic_link))
        .route(
            "/setup",
            get(handlers::setup_get).post(handlers::setup_post),
        )
        .route("/predict/:match_id", axum::routing::post(handlers::predict_match))
        .route("/predict_special", axum::routing::post(handlers::predict_special))
        .route("/leaderboard", get(handlers::leaderboard))
        .route("/profile/jersey-picker", get(handlers::jersey_picker_get))
        .route(
            "/profile/jersey-picker/close",
            get(handlers::jersey_picker_close),
        )
        .route("/profile/jersey", axum::routing::post(handlers::jersey_post))
        .route("/profile/language", axum::routing::post(handlers::set_language_post))
        // Convenience landing — redirects the admin to their own league's
        // user list. Kept so old bookmarks / scripts keep working.
        .route("/admin/users", get(handlers::admin_users_redirect))
        .route(
            "/admin/users/:id/delete",
            axum::routing::post(handlers::admin_delete_user),
        )
        .route(
            "/admin/users/:id/promote",
            axum::routing::post(handlers::admin_toggle_admin),
        )
        .route(
            "/admin/users/:id/rename",
            axum::routing::post(handlers::admin_rename_user),
        )
        .route(
            "/admin/users/:id/resend",
            axum::routing::post(handlers::admin_resend_invite),
        )
        .route(
            "/admin/leagues",
            get(handlers::leagues_list).post(handlers::leagues_create),
        )
        .route("/admin/leagues/new", get(handlers::leagues_new_form))
        .route(
            "/admin/leagues/:id/settings",
            get(handlers::league_settings_form).post(handlers::league_settings_save),
        )
        // User management always lives inside a league scope. Both the league
        // admin (own league) and the super-admin (any league) reach this URL.
        .route(
            "/admin/leagues/:id/users",
            get(handlers::league_users_page).post(handlers::admin_create_user),
        )
        .nest_service("/static", ServeDir::new("static"))
}
