#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
running_pid=
cleanup() {
    status=$?
    touch "$tmp/continue"
    if [[ -n "$running_pid" ]]; then
        wait "$running_pid" || true
    fi
    if (( status != 0 )); then
        cat "$tmp/output.log" >&2 || true
    fi
    rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

mkdir "$tmp/bin" "$tmp/backups"
export BACKUP_DIR="$tmp/backups" BACKUP_TEST_DIR="$tmp"
export BACKUP_TEST_MV
BACKUP_TEST_MV=$(command -v mv)
export HW_BOT_ROOT="$tmp" HW_BOT_IMAGE=example/hw-bot
cat > "$tmp/bin/docker" <<'EOF'
#!/bin/sh
set -eu
while [ "${1:-}" != exec ]; do
    [ "$#" -gt 0 ] || exit 99
    shift
done
shift 3
exec "$@"
EOF
cat > "$tmp/bin/pg_dump" <<'EOF'
#!/bin/sh
set -eu
case "${BACKUP_TEST_MODE:-}" in
    dump_failure) exit 42 ;;
    timeout) exec sleep 30 ;;
    blocked)
        touch "$BACKUP_TEST_DIR/ready"
        while [ ! -f "$BACKUP_TEST_DIR/continue" ]; do sleep 0.05; done
        ;;
esac
printf 'complete dump\n'
EOF
cat > "$tmp/bin/pg_restore" <<'EOF'
#!/bin/sh
set -eu
cat > /dev/null
[ "${BACKUP_TEST_MODE:-}" != restore_failure ] || exit 43
EOF
cat > "$tmp/bin/mv" <<'EOF'
#!/bin/sh
set -eu
if [ "$2" = "$BACKUP_DIR/.last-success" ]; then
    case "${BACKUP_TEST_MODE:-}" in
        before_marker) kill -HUP "$PPID"; exit 0 ;;
        after_marker)
            "$BACKUP_TEST_MV" "$@"
            kill -"$BACKUP_TEST_SIGNAL" "$PPID"
            exit 0
            ;;
    esac
fi
exec "$BACKUP_TEST_MV" "$@"
EOF
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH"
export POSTGRES_USER=hw_bot POSTGRES_DB=hw_bot

backup() {
    sh "$root/dist/bin/backup.sh"
}
verify_backup() {
    local dump partial
    dump=$(cat "$BACKUP_DIR/.last-success")
    [[ -f "$dump" && -f "$dump.sha256" ]]
    (cd "$BACKUP_DIR" && sha256sum --check "${dump##*/}.sha256" > /dev/null)
    flock -n "$BACKUP_DIR/.backup.lock" true
    for partial in "$BACKUP_DIR/"*.partial; do
        [[ ! -e "$partial" ]]
    done
}
expect_failure() {
    local expected=$1 status
    shift
    if "$@" > "$tmp/output.log" 2>&1; then
        printf '%s\n' 'Expected backup to fail' >&2
        exit 1
    else
        status=$?
    fi
    [[ "$status" == "$expected" ]]
}

printf '%s\n' 'Backup publishes a valid dump and checksum'
backup > "$tmp/output.log"
verify_backup
cmp "$tmp/output.log" "$BACKUP_DIR/.last-success"
cp "$BACKUP_DIR/.last-success" "$tmp/previous"

for mode in dump_failure restore_failure timeout before_marker; do
    printf 'Backup preserves the previous marker after %s\n' "$mode"
    case "$mode" in
        dump_failure) expected=42 ;;
        restore_failure) expected=43 ;;
        timeout) expected=124 ;;
        before_marker) expected=1 ;;
    esac
    expect_failure "$expected" env BACKUP_TEST_MODE="$mode" BACKUP_TIMEOUT_SECONDS=1 \
        sh "$root/dist/bin/backup.sh"
    cmp "$tmp/previous" "$BACKUP_DIR/.last-success"
    verify_backup
done

for signal in HUP TERM; do
    printf 'Backup keeps published files after %s\n' "$signal"
    cp "$BACKUP_DIR/.last-success" "$tmp/previous"
    expect_failure 1 env BACKUP_TEST_MODE=after_marker BACKUP_TEST_SIGNAL="$signal" \
        sh "$root/dist/bin/backup.sh"
    [[ "$(cat "$tmp/previous")" != "$(cat "$BACKUP_DIR/.last-success")" ]]
    verify_backup
done

printf '%s\n' 'Concurrent backup is rejected while the first dump is running'
BACKUP_TEST_MODE=blocked BACKUP_TIMEOUT_SECONDS=10 backup > "$tmp/output.log" 2>&1 &
running_pid=$!
for ((attempt = 0; attempt < 100; attempt++)); do
    [[ -f "$tmp/ready" ]] && break
    sleep 0.05
done
[[ -f "$tmp/ready" ]]
expect_failure 1 backup
touch "$tmp/continue"
wait "$running_pid"
running_pid=
verify_backup

for value in 0 00 -1 1s; do
    printf 'Backup rejects invalid timeout %s\n' "$value"
    expect_failure 1 env BACKUP_TIMEOUT_SECONDS="$value" sh "$root/dist/bin/backup.sh"
done

printf '%s\n' 'Backup runs again after failures and interruptions'
backup > "$tmp/output.log"
verify_backup
