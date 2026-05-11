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
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

# Create unprivileged user – the binary must never run as root.
RUN groupadd -r appuser && useradd -r -g appuser appuser

COPY --from=builder --chown=appuser:appuser /app/target/release/pila .

USER appuser

EXPOSE 8000

# TODO: add a GET /healthz route so the container runtime can probe liveness.
# HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
#   CMD ["/app/pila", "--healthcheck"]

CMD ["./pila"]
