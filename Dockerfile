FROM rust:1.98.1-bookworm AS builder

WORKDIR /src
COPY . .

ENV SQLX_OFFLINE=true
RUN cargo build --locked --release --bin hw-bot

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /app hw-bot

WORKDIR /app
COPY --from=builder /src/target/release/hw-bot /usr/local/bin/hw-bot
COPY config/default.toml /app/config/default.toml
COPY --chmod=0755 tools/docker/entrypoint.sh /usr/local/bin/hw-bot-entrypoint

USER 10001:10001

HEALTHCHECK --interval=10s --timeout=6s --start-period=20s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 5 \
        http://127.0.0.1:3000/health/ready

ENTRYPOINT ["/usr/local/bin/hw-bot-entrypoint"]
