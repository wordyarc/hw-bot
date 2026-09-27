#!/bin/sh
set -eu
umask 077

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
export HW_BOT_ENV_FILE="${HW_BOT_ENV_FILE:-"$root/tools/deploy/.env.production"}"
export HW_BOT_CONFIG_DIR="${HW_BOT_CONFIG_DIR:-"$root/tools/deploy"}"
backup_timeout=${BACKUP_TIMEOUT_SECONDS:-300}
case "$backup_timeout" in
    ''|*[!0-9]*) printf '%s\n' 'BACKUP_TIMEOUT_SECONDS must be a positive integer' >&2; exit 1 ;;
esac
[ "$backup_timeout" -gt 0 ] || { printf '%s\n' 'BACKUP_TIMEOUT_SECONDS must be a positive integer' >&2; exit 1; }
backup_dir=${BACKUP_DIR:-"$root/backups"}
mkdir -p "$backup_dir"
backup_dir=$(CDPATH='' cd -- "$backup_dir" && pwd)
exec 9> "$backup_dir/.backup.lock"
flock -n 9 || { printf '%s\n' 'Another backup is running' >&2; exit 1; }

dump="$backup_dir/hw-bot-$(date -u +%Y%m%dT%H%M%SZ)-$$.dump"
marker="$backup_dir/.last-success.$$"
cleanup() {
    rm -f "$dump.partial" "$dump.sha256.partial" "$marker"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

compose() {
    docker compose --env-file "$HW_BOT_ENV_FILE" -f tools/deploy/compose.yml "$@"
}
compose exec -T database timeout --verbose --kill-after=10s "${backup_timeout}s" \
    sh -c "exec pg_dump --username=\"\$POSTGRES_USER\" --dbname=\"\$POSTGRES_DB\" \
        --format=custom --compress=zstd:6 --lock-wait-timeout=10s" \
    > "$dump.partial"
compose exec -T database pg_restore --list < "$dump.partial" > /dev/null
checksum=$(sha256sum "$dump.partial")
printf '%s  %s\n' "${checksum%% *}" "${dump##*/}" > "$dump.sha256.partial"
printf '%s\n' "$dump" > "$marker"
mv "$dump.partial" "$dump"
mv "$dump.sha256.partial" "$dump.sha256"
mv "$marker" "$backup_dir/.last-success"
printf '%s\n' "$dump"
