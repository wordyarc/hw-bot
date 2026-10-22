![HW·bot Logo (Light)](docs/logo/svg/hw-bot-lockup-en.svg#gh-light-mode-only)
![HW·bot Logo (Dark)](docs/logo/svg/hw-bot-lockup-en-dark.svg#gh-dark-mode-only)

# hw-bot

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/WordyArc/hw-bot/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/WordyArc/hw-bot/actions/workflows/ci.yml)
[![Release](https://github.com/WordyArc/hw-bot/actions/workflows/release.yml/badge.svg)](https://github.com/WordyArc/hw-bot/actions/workflows/release.yml)
[![Docker Image](https://img.shields.io/docker/v/wordyarc/hw-bot?sort=semver&label=docker)](https://hub.docker.com/r/wordyarc/hw-bot)
[![Rust 1.99](https://img.shields.io/badge/Rust-1.99-orange.svg?logo=rust)](https://www.rust-lang.org)
[![PostgreSQL 18](https://img.shields.io/badge/PostgreSQL-18-336791.svg?logo=postgresql&logoColor=white)](https://www.postgresql.org)

A Telegram bot for submitting and reviewing homework.

Students submit work through the bot. Teachers go through a queue of submissions in the order they
arrived and accept or reject each one with a comment. The bot also handles deadlines, late
penalties, makeup submissions and course gradebooks.

Rust · axum · teloxide · PostgreSQL · sqlx

## Local setup

Start PostgreSQL:

```bash
docker compose -f tools/deploy/compose.local.yml up -d
```

Create `config/local.toml`:

```toml
[telegram]
enabled = true
bot_token = "123456:token-from-botfather"

[group_invites]
# base64url of 32 random bytes, no padding: openssl rand -base64 32 | tr '+/' '-_' | tr -d '='
invite_signing_secret = ""

[notification_worker]
enabled = true
```

Run:

```bash
cargo run
```

Configuration is loaded from `config/default.toml`, then `config/local.toml`, then `APP__*`
environment variables.

## Tests

```bash
cargo test
```

Docker is required for the integration tests.
