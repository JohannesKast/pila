# Build stage
FROM rust:1-slim as builder

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev curl && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
RUN mkdir src
RUN echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src/

COPY . .

# Compile the production CSS bundle (static/app.css) with the standalone
# Tailwind CLI — no Node toolchain in the image. Regenerated on every build so
# it can never drift from the templates. Keep TAILWIND_VERSION in sync with the
# devDependency in package.json (used for local `npm run build:css`).
ARG TAILWIND_VERSION=v3.4.17
RUN arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) tw_arch=x64 ;; \
      arm64) tw_arch=arm64 ;; \
      *) echo "unsupported arch for Tailwind CLI: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSLo /usr/local/bin/tailwindcss \
      "https://github.com/tailwindlabs/tailwindcss/releases/download/${TAILWIND_VERSION}/tailwindcss-linux-${tw_arch}" \
    && chmod +x /usr/local/bin/tailwindcss
RUN tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify

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
