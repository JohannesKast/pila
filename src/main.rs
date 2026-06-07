// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Johannes Kast

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::{routing::get, Router};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use std::sync::Arc;

use pila::handlers;
use pila::news;
use pila::repo::Repos;
use pila::scoreboard::EspnClient;
use pila::{time, worker, AppState};

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
        .acquire_timeout(std::time::Duration::from_secs(3))
        .idle_timeout(std::time::Duration::from_secs(300))
        .max_lifetime(std::time::Duration::from_secs(1800))
        .connect(&db_url)
        .await
        .unwrap();

    tracing::info!("Running database migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run DB migrations");

    let repos = Repos::from_pool(pool.clone());

    // Read once at startup so handlers and the worker don't call
    // `std::env::var` on every request / notification tick.
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8000".into());
    let signal_api_url = std::env::var("SIGNAL_API_URL").ok();
    let signal_from_number = std::env::var("SIGNAL_FROM_NUMBER").ok();
    let signal_group_id = std::env::var("SIGNAL_GROUP_ID").ok();

    let http_client = reqwest::Client::new();

    let smtp_config = pila::mail::SmtpConfig::from_env();
    if smtp_config.is_some() {
        tracing::info!("SMTP configured — email delivery enabled");
    } else {
        tracing::info!("SMTP env vars not set — email delivery disabled");
    }

    // Dev mode enables the /dev/* routes for testing. Must be explicitly enabled.
    let dev_mode = std::env::var("PILA_DEV_MODE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    if dev_mode {
        tracing::warn!(
            "⚠️  PILA_DEV_MODE enabled — dev testing routes are active. Do not use in production!"
        );
    }

    // Mock time storage. None = use real time. Only set in dev mode.
    let mock_now = time::new_mock_time();

    let state = AppState {
        jerseys: pila::jersey::load(),
        news: news::NewsCache::from_env(),
        repos: repos.clone(),
        translations: pila::translations::load_all(),
        concurrency_limit: Arc::new(tokio::sync::Semaphore::new(pila::MAX_CONCURRENT_REQUESTS)),
        base_url: base_url.clone(),
        signal_api_url: signal_api_url.clone(),
        signal_from_number: signal_from_number.clone(),
        signal_group_id: signal_group_id.clone(),
        http_client: http_client.clone(),
        mock_now,
        dev_mode,
        smtp_config: smtp_config.clone(),
    };

    if let Err(e) = worker::bootstrap_notifications(&repos).await {
        tracing::warn!("Notification bootstrap failed: {:?}", e);
    }

    let scoreboard: Arc<dyn pila::scoreboard::ScoreboardClient> = Arc::new(EspnClient::new());
    if dev_mode {
        // One-shot sync — and only when the matches table is empty. The
        // upsert does `status = EXCLUDED.status` (not COALESCE), so a sync
        // would revert any manually-set "finished" status back to ESPN's
        // "scheduled" for future matches. Skipping the sync when data
        // exists preserves dev state across server restarts.
        let already_seeded = repos.matches.first_kickoff().await.ok().flatten().is_some();
        if already_seeded {
            tracing::info!(
                "Dev mode: matches already seeded, skipping ESPN sync to preserve state"
            );
        } else {
            match worker::update_data(&*scoreboard, &repos).await {
                Ok(_) => tracing::info!("Dev one-shot scoreboard sync complete"),
                Err(e) => tracing::warn!("Dev one-shot scoreboard sync failed: {:?}", e),
            }
        }
    } else {
        worker::start_background_worker(
            repos,
            scoreboard,
            base_url,
            signal_api_url,
            smtp_config,
            state.translations.clone(),
        )
        .await;
    }

    let app = build_router();
    let app = if dev_mode {
        app.merge(build_dev_router())
    } else {
        app
    };
    let app = app
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state,
            concurrency_limit_middleware,
        ));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".into());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().expect("Invalid PORT");
    tracing::info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

/// Set security-related HTTP response headers on every response.
async fn security_headers_middleware(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    use axum::http::HeaderValue;
    headers.insert(
        "Strict-Transport-Security",
        HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );

    response
}

/// Validates the `X-CSRF-Token` request header against the `pila_csrf`
/// cookie (double-submit-cookie pattern) for state-changing POST routes.
/// Exempts `/setup` (no cookie yet) and `/play/me/*` (login flow).
async fn csrf_middleware(
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.method() != axum::http::Method::POST {
        return Ok(next.run(request).await);
    }

    let path = request.uri().path();
    if path == "/setup"
        || path.starts_with("/play/me/")
        || path.starts_with("/join/")
        || path.starts_with("/dev")
    {
        return Ok(next.run(request).await);
    }

    let jar = axum_extra::extract::CookieJar::from_headers(request.headers());
    let cookie_token = jar.get("pila_csrf").map(|c| c.value().to_owned());

    let header_token = request
        .headers()
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    match (cookie_token, header_token) {
        (Some(c), Some(h)) if c == h => Ok(next.run(request).await),
        _ => Err(StatusCode::FORBIDDEN),
    }
}

/// Waits for SIGTERM (Unix) or SIGINT (Ctrl+C), then returns so
/// `axum::serve().with_graceful_shutdown()` can drain connections.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, starting graceful shutdown...");
}

/// Global concurrency gate.  Acquires one permit from the shared semaphore
/// before every request and releases it when the response is sent.
/// Requests beyond `MAX_CONCURRENT_REQUESTS` queue up, providing
/// backpressure without dropping connections.
async fn concurrency_limit_middleware(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let _permit = state
        .concurrency_limit
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(next.run(request).await)
}

/// Wire up every route. Pulled out of `main` so integration tests can spin
/// up a real `axum::Router` against a fake `AppState` (see `tests/`).
///
/// The concurrency-limiting middleware is applied in `main()` *after*
/// `with_state` so the semaphore is truly global (shared across all TCP
/// connections) rather than per-connection.
fn build_router() -> Router<AppState> {
    Router::new()
        .layer(middleware::from_fn(security_headers_middleware))
        .layer(middleware::from_fn(csrf_middleware))
        .route("/", get(handlers::index))
        .route("/healthz", get(handlers::healthz))
        .route("/play/me/{token}", get(handlers::login_magic_link))
        .route(
            "/join/{token}",
            get(handlers::join_get).post(handlers::join_post),
        )
        .route(
            "/setup",
            get(handlers::setup_get).post(handlers::setup_post),
        )
        .route(
            "/predict/{match_id}",
            axum::routing::post(handlers::predict_match),
        )
        .route(
            "/predict_special",
            axum::routing::post(handlers::predict_special),
        )
        .route("/leaderboard", get(handlers::leaderboard))
        .route("/profile/jersey-picker", get(handlers::jersey_picker_get))
        .route(
            "/profile/jersey-picker/close",
            get(handlers::jersey_picker_close),
        )
        .route(
            "/profile/jersey",
            axum::routing::post(handlers::jersey_post),
        )
        .route(
            "/profile/language",
            axum::routing::post(handlers::set_language_post),
        )
        // Convenience landing — redirects the admin to their own league's
        // user list. Kept so old bookmarks / scripts keep working.
        .route("/admin/users", get(handlers::admin_users_redirect))
        .route(
            "/admin/users/{id}/delete",
            axum::routing::post(handlers::admin_delete_user),
        )
        .route(
            "/admin/users/{id}/promote",
            axum::routing::post(handlers::admin_toggle_admin),
        )
        .route(
            "/admin/users/{id}/rename",
            axum::routing::post(handlers::admin_rename_user),
        )
        .route(
            "/admin/users/{id}/resend",
            axum::routing::post(handlers::admin_resend_invite),
        )
        .route(
            "/admin/leagues/{id}/invites",
            axum::routing::post(handlers::admin_create_invite),
        )
        .route(
            "/admin/invites/{id}/revoke",
            axum::routing::post(handlers::admin_revoke_invite),
        )
        .route(
            "/admin/leagues",
            get(handlers::leagues_list).post(handlers::leagues_create),
        )
        .route("/admin/leagues/new", get(handlers::leagues_new_form))
        .route(
            "/admin/leagues/{id}/settings",
            get(handlers::league_settings_form).post(handlers::league_settings_save),
        )
        // User management always lives inside a league scope. Both the league
        // admin (own league) and the super-admin (any league) reach this URL.
        .route(
            "/admin/leagues/{id}/users",
            get(handlers::league_users_page).post(handlers::admin_create_user),
        )
        .nest_service("/static", ServeDir::new("static"))
}

/// Dev/testing routes. Only mounted when `PILA_DEV_MODE=true`.
/// These routes allow simulating tournament progression without real API data.
fn build_dev_router() -> Router<AppState> {
    Router::new()
        // Exempt dev routes from CSRF checks (they're for testing only)
        .layer(middleware::from_fn(security_headers_middleware))
        .route("/dev", get(handlers::dev_panel))
        .route("/dev/time", axum::routing::post(handlers::dev_set_time))
        .route(
            "/dev/time/reset",
            axum::routing::post(handlers::dev_reset_time),
        )
        .route(
            "/dev/tips/random",
            axum::routing::post(handlers::dev_random_tips),
        )
        .route(
            "/dev/tips/all-users",
            axum::routing::post(handlers::dev_random_tips_all_users),
        )
        .route(
            "/dev/results/random",
            axum::routing::post(handlers::dev_random_results),
        )
        .route(
            "/dev/simulate/next-matchday",
            axum::routing::post(handlers::dev_simulate_next_matchday),
        )
        .route("/dev/users", get(handlers::dev_list_users))
        .route(
            "/dev/switch-user/{id}",
            axum::routing::post(handlers::dev_switch_user),
        )
}
