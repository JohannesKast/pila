# Pila Security & Code Quality Audit – TODO

## 🔴 Kritisch / Hoch (vor Deployment fixen)

- [ ] **Rate-Limiting für alle POST-Routen**  
  `src/main.rs` — `tower::limit::RateLimitLayer` oder `tower_http::limit::RequestBodyLimitLayer` auf Router oder pro Route.  
  Besonders: `/predict/:match_id`, `/setup`, `/predict_special`, alle `/admin/*` POSTs.

- [ ] **Security-Header (CSP, HSTS, X-Frame-Options, X-Content-Type-Options)**  
  `src/main.rs` — `tower_http::set_header::SetResponseHeaderLayer` Middleware.  
  `templates/base.html` — `<meta>`-Fallback oder CSP-Header ergänzen (externe CDNs: tailwindcss, htmx, fonts).

- [ ] **Graceful Shutdown**  
  `src/main.rs` — `axum::serve()` mit `.with_graceful_shutdown()` + SIGTERM-Handler.  
  Signal-Closure: `tokio::signal::ctrl_c()` + `#[cfg(unix)] SIGTERM`.

- [ ] **`/healthz`-Route**  
  `src/main.rs` — `build_router()`: einfache `GET /healthz` → `"OK"`.  
  `Dockerfile` — `HEALTHCHECK`-Instruction einkommentieren.  
  `docker-compose.yml` — `healthcheck` für `app`-Service ergänzen.

- [ ] **CSRF-Schutz auf State-Changing POSTs prüfen**  
  HTMX-basierte POSTs: `SameSite=Lax` reicht teilweise, aber kein expliziter Token.  
  Entweder: `tower_http::csrf` (wenn verfügbar) oder manuelles Double-Submit-Cookie-Pattern.

## 🟡 Mittel

- [ ] **`setup_post`: Liga + Settings + User in eine DB-Transaktion packen**  
  `src/handlers/setup.rs` — `pool.begin().await?` / `tx.commit().await?` um `league_create`, `persist_setting` ×4, `user_create`, `set_can_create_league`.

- [ ] **`unwrap()` auf `Template::render()` durch `.map_err()` ersetzen**  
  Alle Handler-Dateien: `admin.rs`, `leaderboard.rs`, `jersey.rs`, `leagues.rs`, `setup.rs`, `index.rs`.  
  Mapping: `render().map_err(|e| { tracing::error!(%e, "template error"); (500, "Interner Fehler") })`.

- [ ] **sqlx 0.7 → 0.8 upgraden**  
  `Cargo.toml`: `sqlx = { version = "0.8" }`.  
  Danach `cargo sqlx prepare` neu ausführen und `.sqlx/` committen.

- [ ] **`tokio` Feature-Set einschränken**  
  `Cargo.toml`: `"full"` → `["rt-multi-thread", "macros", "net", "time", "sync", "signal"]`.

- [ ] **`BASE_URL` / Signal-Konfig in AppState cachen**  
  `src/lib.rs`: `AppState` um Felder `base_url: String`, `signal_api_url: Option<String>`, … erweitern.  
  `src/main.rs`: einmal beim Startup aus Env lesen.  
  `handlers/util.rs`, `notifier.rs`: von `State(state)` lesen statt `std::env::var()`.

- [ ] **Pool-Timeout-Konfiguration**  
  `src/main.rs`: `PgPoolOptions` um `.acquire_timeout(3s)`, `.idle_timeout(300s)`, `.max_lifetime(1800s)` ergänzen.

- [ ] **Docker: `pids_limit` + `mem_swappiness`**  
  `docker-compose.yml` → `app`-Service: `pids_limit: 100`, `mem_swappiness: 0`.

- [ ] **`main.rs`: `[IP_ADDRESS]`-Literal korrigieren**  
  Zeile 58: `[IP_ADDRESS]` → `0.0.0.0` (oder `[::0]`). Der Wert ist aktuell tatsächlich `[IP_ADDRESS]`, kein Platzhalter.

## 🟢 Niedrig

- [ ] **Clippy-Check in CI / Pre-Commit**  
  `Cargo.toml`: `[lints.clippy]` Sektion oder `#![deny(clippy::unwrap_used)]` in `lib.rs`.  
  Optional: `.github/workflows/ci.yml` mit `cargo clippy -- -D warnings`.

- [ ] **Inline-HTML-Fragment in `predictions.rs` → Askama-Template**  
  `src/handlers/predictions.rs:66-74` — `format!(r##"<form …>"##)` durch eigenes `predict_form.html` ersetzen.

- [ ] **Askama `unwrap()` in `t_for()` Helper**  
  `src/handlers/admin.rs:37-44` + gleiche Pattern in `leaderboard.rs`, `jersey.rs`, etc.  
  Duplizierten Code in `util.rs` zentralisieren und `.expect()` mit Kontext versehen.

- [ ] **`reqwest::Client` als Singleton**  
  `src/notifier.rs` — `Client::new()` wird bei jedem `notify()` neu erstellt. Einmal bauen und in `AppState` halten.

- [ ] **Signal REST API: TLS dokumentieren**  
  Aktuell `http://signal-cli:8080` — im internen Docker-Netzwerk OK, aber in `docker-compose.yml` als Kommentar vermerken.

- [ ] **`RUST_LOG` default nicht in `main.rs` hartkodieren**  
  `tracing_subscriber::EnvFilter::new("info")` — sollte `std::env::var("RUST_LOG").unwrap_or("info")` verwenden (wird bereits gemacht — check).

- [ ] **Docker: Logging-Driver konfigurieren**  
  `docker-compose.yml` → `app`-Service: `logging:` mit `driver: json-file`, `max-size: 10m`, `max-file: 3`.

## Geschätzte Aufwände

| Bereich | Punkte | Zeit |
|---------|--------|------|
| 🔴 Kritisch/Hoch | 5 | ~1 h |
| 🟡 Mittel | 8 | ~1,5 h |
| 🟢 Niedrig | 8 | ~1 h |
| **Gesamt** | **21** | **~3,5 h** |
