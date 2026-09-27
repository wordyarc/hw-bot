#!/bin/sh
set -eu
umask 077

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
export HW_BOT_ENV_FILE="${HW_BOT_ENV_FILE:-"$root/tools/deploy/.env.production"}"
export HW_BOT_CONFIG_DIR="${HW_BOT_CONFIG_DIR:-"$root/tools/deploy"}"
backup_dir=${BACKUP_DIR:-"$root/backups"}
mkdir -p "$backup_dir"
backup_dir=$(CDPATH='' cd -- "$backup_dir" && pwd)
lock="$backup_dir/.backup-lock"
if ! mkdir "$lock"; then
    printf '%s\n' "Backup is already running or a stale lock exists: $lock" >&2
    exit 1
fi

dump="$backup_dir/hw-bot-$(date -u +%Y%m%dT%H%M%SZ)-$$.dump"
marker="$backup_dir/.last-success.$$"
completed=false
cleanup() {
    rm -f "$dump.partial" "$dump.sha256.partial" "$marker"
    if [ "$completed" = false ]; then
        rm -f "$dump" "$dump.sha256"
    fi
    rmdir "$lock"
}
trap cleanup EXIT
trap 'exit 1' INT TERM

compose() {
    docker compose --env-file "$HW_BOT_ENV_FILE" -f tools/deploy/compose.yml "$@"
}
compose exec -T database sh -c \
    "pg_dump --username=\"\$POSTGRES_USER\" --dbname=\"\$POSTGRES_DB\" --format=custom --compress=zstd:6" \
    > "$dump.partial"
compose exec -T database pg_restore --list < "$dump.partial" > /dev/null
checksum=$(sha256sum "$dump.partial")
printf '%s  %s\n' "${checksum%% *}" "${dump##*/}" > "$dump.sha256.partial"
printf '%s\n' "$dump" > "$marker"
mv "$dump.partial" "$dump"
mv "$dump.sha256.partial" "$dump.sha256"
mv "$marker" "$backup_dir/.last-success"
completed=true
printf '%s\n' "$dump"
