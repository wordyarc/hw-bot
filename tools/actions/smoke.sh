#!/bin/sh
set -eu
umask 077

image=${1:?usage: sh tools/actions/smoke.sh IMAGE}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/hw-bot-smoke.XXXXXX")
diagnostics=${2:-"$tmp/diagnostics"}
export COMPOSE_PROJECT_NAME="hw-bot-smoke-$$"
export HW_BOT_ENV_FILE="$tmp/.env.production" HW_BOT_CONFIG_DIR="$tmp"
export HW_BOT_IMAGE="$image" POSTGRES_PASSWORD=smoke
compose() {
    docker compose --env-file "$HW_BOT_ENV_FILE" -f tools/deploy/compose.yml "$@"
}
cleanup() {
    status=$?
    set +e
    if [ "$status" -ne 0 ]; then
        mkdir -p "$diagnostics"
        compose ps --all > "$diagnostics/compose-ps.log" 2>&1 || true
        compose logs --no-color --tail 200 app database > "$diagnostics/compose.log" 2>&1 || true
        for container in $(compose ps --all -q); do
            docker inspect --format '{{json .State}}' "$container" > "$diagnostics/$container-state.json" 2>&1 || true
        done
        if [ -f "$tmp/startup.log" ]; then cp "$tmp/startup.log" "$diagnostics/startup.log"; fi
        cat "$diagnostics/compose-ps.log" "$diagnostics/compose.log" >&2
    fi
    compose down --volumes --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$tmp"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 1' INT TERM

sed '/^\[server\]$/,/^\[/ {
    s/^host = .*/host = "::1"/
    s/^port = .*/port = 3001/
}' tools/deploy/production.example.toml > "$tmp/production.toml"
grep -qx 'host = "::1"' "$tmp/production.toml"
grep -qx 'port = 3001' "$tmp/production.toml"
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

backup=$(BACKUP_DIR="$tmp/backups" sh tools/ops/backup.sh)
[ "$(cat "$tmp/backups/.last-success")" = "$backup" ]
(cd "$tmp/backups" && sha256sum --check "${backup##*/}.sha256")
compose exec -T database createdb --username=hw_bot restore_check
compose exec -T database pg_restore --username=hw_bot --dbname=restore_check \
    --no-owner --exit-on-error < "$backup"

for config in missing directory unreadable; do
    case "$config" in
        missing) set -- "$image" ;;
        directory) set -- --volume "$tmp:/app/config/local.toml:ro" "$image" ;;
        unreadable)
            set -- --user 0 --entrypoint sh "$image" -ec \
                'touch /app/config/local.toml; chmod 0000 /app/config/local.toml; exec su -s /bin/sh -c /usr/local/bin/hw-bot-entrypoint hw-bot'
            ;;
    esac
    if docker run --rm --network none "$@" > "$tmp/startup.log" 2>&1; then
        printf 'Startup accepted %s production config\n' "$config" >&2
        exit 1
    fi
    if ! grep -q 'Production config /app/config/local.toml must be a readable file' "$tmp/startup.log"; then
        cat "$tmp/startup.log" >&2
        exit 1
    fi
done
