FROM rust:1.99.0-slim-bookworm AS chef

RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /src

FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

ENV SQLX_OFFLINE=true
COPY rust-toolchain.toml ./
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --locked --release --bin hw-bot --recipe-path recipe.json

COPY . .
RUN cargo build --locked --release --bin hw-bot \
    && cp target/release/hw-bot /usr/local/bin/hw-bot \
    && rm -rf target

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /app hw-bot

WORKDIR /app
COPY --from=builder /usr/local/bin/hw-bot /usr/local/bin/hw-bot
COPY config/default.toml /app/config/default.toml
COPY --chmod=0755 tools/docker/entrypoint.sh /usr/local/bin/hw-bot-entrypoint

USER 10001:10001

HEALTHCHECK --interval=10s --timeout=6s --start-period=20s --start-interval=1s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 5 \
        http://127.0.0.1:3000/health/ready

ENTRYPOINT ["/usr/local/bin/hw-bot-entrypoint"]
