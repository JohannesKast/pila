# Build stage
FROM rust:1-slim as builder

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
RUN mkdir src
RUN echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src/

COPY . .

ENV SQLX_OFFLINE=true

RUN touch src/main.rs
RUN cargo build --release

# Run stage
FROM debian:bookworm-slim

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 curl && rm -rf /var/lib/apt/lists/*

# Create unprivileged user – the binary must never run as root.
RUN groupadd -r appuser && useradd -r -g appuser appuser

# Runtime resources read from disk by the binary (relative to WORKDIR):
#   - locales/  → translations::load_at_startup (std::fs::read_to_string)
#   - static/   → ServeDir::new("static") in build_router
# Templates and migrations are embedded into the binary at compile time
# (Askama, sqlx::migrate!) and therefore need no COPY here.
COPY --from=builder --chown=appuser:appuser /app/target/release/pila .
COPY --from=builder --chown=appuser:appuser /app/locales ./locales
COPY --from=builder --chown=appuser:appuser /app/static ./static

USER appuser

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["curl", "-f", "http://127.0.0.1:8000/healthz"]

CMD ["./pila"]
