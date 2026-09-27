#!/bin/sh
set -eu
umask 077

image=${1:?usage: sh tools/ci/smoke.sh IMAGE}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/hw-bot-smoke.XXXXXX")
export COMPOSE_PROJECT_NAME="hw-bot-smoke-$$"
export HW_BOT_ENV_FILE="$tmp/.env.production" HW_BOT_CONFIG_DIR="$tmp"
export HW_BOT_IMAGE="$image" POSTGRES_PASSWORD=smoke
compose() {
    docker compose --env-file "$HW_BOT_ENV_FILE" -f tools/deploy/compose.yml "$@"
}
cleanup() {
    status=$?
    if [ "$status" -ne 0 ]; then
        compose logs --no-color --tail 100 app || true
    fi
    compose down --volumes --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' INT TERM

cp tools/deploy/production.example.toml "$tmp/production.toml"
cp tools/deploy/bootstrap.example.json "$tmp/bootstrap.production.json"
chmod 0640 "$tmp/production.toml" "$tmp/bootstrap.production.json"
cat > "$HW_BOT_ENV_FILE" <<EOF
HW_BOT_IMAGE=$image
HW_BOT_CONFIG_GID=$(id -g)
POSTGRES_PASSWORD=smoke
APP__DATABASE__URL=postgres://hw_bot:smoke@database:5432/hw_bot
APP__TELEGRAM__ENABLED=false
APP__NOTIFICATION_WORKER__ENABLED=false
APP__GROUP_INVITES__INVITE_SIGNING_SECRET=AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8
EOF
compose -f tools/deploy/compose.bootstrap.yml up -d --wait --wait-timeout 120
compose -f tools/deploy/compose.bootstrap.yml restart app
compose -f tools/deploy/compose.bootstrap.yml up -d --wait --wait-timeout 120
compose up -d --no-deps --force-recreate --wait --wait-timeout 120 app
