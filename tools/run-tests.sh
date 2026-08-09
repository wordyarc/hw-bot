#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

cleanup() {
  docker rm -fv hw-bot-test-postgres >/dev/null 2>&1 || true
}
trap cleanup EXIT

SQLX_OFFLINE=true cargo test
