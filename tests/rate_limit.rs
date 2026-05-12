//! Concurrency-limit middleware tests.
//!
//! `main.rs` applies a global semaphore-based concurrency gate (30 permits)
//! via `middleware::from_fn_with_state`.  These tests exercise the same
//! pattern with a single permit to prove that concurrent requests are
//! serialised — the second request queues behind the semaphore until the
//! first one finishes.
//!
//! Tests start a real TCP listener so the semaphore is exercised
//! end-to-end across separate HTTP connections.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::{routing::get, Router};
use tokio::sync::Semaphore;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Wrapper that mirrors the `concurrency_limit` field on `pila::AppState`.
#[derive(Clone)]
struct TestState {
    semaphore: Arc<Semaphore>,
}

async fn gate_middleware(
    State(state): State<TestState>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let _permit = state
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(next.run(request).await)
}

/// Spawn a router on a random port, run the closure, and tear down.
async fn with_server<F, Fut, T>(router: Router, f: F) -> T
where
    F: FnOnce(SocketAddr) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = axum::serve(listener, router);
    let handle = tokio::spawn(async move { server.await.unwrap() });
    let result = f(addr).await;
    handle.abort();
    result
}

// ─── tests ───────────────────────────────────────────────────────────────────

/// Sequential requests all succeed — no concurrency pressure.
#[tokio::test]
async fn sequential_requests_all_ok() {
    let state = TestState {
        semaphore: Arc::new(Semaphore::new(2)),
    };

    let router = Router::new()
        .route("/", get(|| async { "ok" }))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, gate_middleware));

    with_server(router, |addr| async move {
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/");
        for _ in 0..5 {
            let resp = client.get(&url).send().await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    })
    .await;
}

/// With one permit, two concurrent requests must execute one after the
/// other.  Each handler sleeps 100 ms, so total wall-clock time with one
/// permit must be ≥ 200 ms (serial).  With two permits they'd overlap and
/// finish in ≈ 100 ms (parallel).
#[tokio::test]
async fn single_permit_serialises_two_concurrent_requests() {
    let state = TestState {
        semaphore: Arc::new(Semaphore::new(1)),
    };

    let router: Router = Router::new()
        .route(
            "/",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                "ok"
            }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, gate_middleware));

    with_server(router, |addr| async move {
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/");

        let start = Instant::now();
        let (r1, r2) = tokio::join!(client.get(&url).send(), client.get(&url).send());
        let elapsed = start.elapsed();

        assert_eq!(r1.unwrap().status(), StatusCode::OK);
        assert_eq!(r2.unwrap().status(), StatusCode::OK);

        // 1 permit → 2 × 100 ms must be serial → ≥ 190 ms with some slack.
        assert!(
            elapsed >= Duration::from_millis(190),
            "expected >=190 ms serial execution, got {elapsed:?}",
        );
    })
    .await;
}

/// With two permits both requests overlap and finish in ~100 ms.
#[tokio::test]
async fn two_permits_allows_overlap() {
    let state = TestState {
        semaphore: Arc::new(Semaphore::new(2)),
    };

    let router: Router = Router::new()
        .route(
            "/",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                "ok"
            }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, gate_middleware));

    with_server(router, |addr| async move {
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/");

        let start = Instant::now();
        let (r1, r2) = tokio::join!(client.get(&url).send(), client.get(&url).send());
        let elapsed = start.elapsed();

        assert_eq!(r1.unwrap().status(), StatusCode::OK);
        assert_eq!(r2.unwrap().status(), StatusCode::OK);

        // 2 permits → both can run in parallel → < 190 ms.
        assert!(
            elapsed < Duration::from_millis(190),
            "expected <190 ms parallel execution, got {elapsed:?}",
        );
    })
    .await;
}
