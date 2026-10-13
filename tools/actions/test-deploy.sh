#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
cleanup() {
    status=$?
    if (( status != 0 )); then cat "$tmp/output.log" >&2 || true; fi
    rm -rf "$tmp"
}
trap cleanup EXIT

export DEPLOY_TEST_ROOT="$tmp/installation" DEPLOY_TEST_LOG="$tmp/calls.log"
export DEPLOY_TEST_TAR DEPLOY_TEST_MV
DEPLOY_TEST_TAR=$(command -v tar)
DEPLOY_TEST_MV=$(command -v mv)
mkdir -p "$tmp/bin" "$tmp/package/dist/bin" "$tmp/release"
files=(Makefile bin/backup.sh bin/compose.sh bin/deploy.sh compose.yml)
for file in "${files[@]}"; do printf 'new file\n' > "$tmp/package/dist/$file"; done
cp "$root/dist/bin/compose.sh" "$tmp/package/dist/bin/compose.sh"
cat > "$tmp/package/dist/bin/backup.sh" <<'EOF'
#!/bin/sh
printf 'backup\n' >> "$DEPLOY_TEST_LOG"
[ "$DEPLOY_TEST_MODE" != backup_failure ] || exit 42
printf '/backups/before-deploy.dump\n'
EOF
tar -czf "$tmp/release/deploy.tar.gz" -C "$tmp/package" dist
tar -xzf "$tmp/release/deploy.tar.gz" -C "$tmp/release" --strip-components=1
image="example/hw-bot@sha256:$(printf '%064d' 1)"
jq -n --arg image "$image" '{image: $image}' > "$tmp/release/release.json"
cat > "$tmp/bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == compose ]]; then
    shift
    while [[ "$1" == -* ]]; do shift 2; done
fi
printf '%s\n' "$*" >> "$DEPLOY_TEST_LOG"
case "$1" in
    ps) [[ "$*" != 'ps --all -q app' ]] || printf 'old-container\n' ;;
    inspect) cat "$DEPLOY_TEST_ROOT/image" ;;
    config|pull|logs) ;;
    stop)
        printf 'stopped\n' > "$DEPLOY_TEST_ROOT/state"
        case "$DEPLOY_TEST_MODE" in
            hup_stop) kill -HUP "$PPID" ;;
            term_stop) kill -TERM "$PPID" ;;
            timeout_stop) sleep 30 ;;
        esac
        ;;
    start)
        [[ "$2" == old-container ]]
        printf 'running\n' > "$DEPLOY_TEST_ROOT/state"
        ;;
    up)
        [[ "$(cat "$DEPLOY_TEST_ROOT/current-release.json")" == 'previous release' ]]
        printf 'starting\n' > "$DEPLOY_TEST_ROOT/state"
        printf '%s\n' "$HW_BOT_IMAGE" > "$DEPLOY_TEST_ROOT/image"
        [[ "$DEPLOY_TEST_MODE" != startup_failure ]] || exit 43
        case "$DEPLOY_TEST_MODE" in
            term_start) kill -TERM "$PPID"; exit 0 ;;
            timeout_start) sleep 30 ;;
        esac
        printf 'new\n' > "$DEPLOY_TEST_ROOT/state"
        ;;
    *) exit 99 ;;
esac
EOF
cat > "$tmp/bin/tar" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$DEPLOY_TEST_MODE" == write_failure ]]; then
    while [[ "$1" != -C ]]; do shift; done
    printf 'partial\n' > "$2/compose.yml"
    exit 44
fi
exec "$DEPLOY_TEST_TAR" "$@"
EOF
cat > "$tmp/bin/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$DEPLOY_TEST_MODE:$2" in
    "rename_failure:$DEPLOY_TEST_ROOT/Makefile"|\
    "late_rename_failure:$DEPLOY_TEST_ROOT/compose.yml"|\
    "record_failure:$DEPLOY_TEST_ROOT/current-release.json") exit 45 ;;
esac
exec "$DEPLOY_TEST_MV" "$@"
EOF
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH"

for mode in missing_logs write_failure backup_failure rename_failure late_rename_failure \
    hup_stop term_stop timeout_stop startup_failure term_start timeout_start record_failure success; do
    printf 'Deployment handles %s\n' "$mode"
    export DEPLOY_TEST_MODE="$mode"
    rm -rf "$DEPLOY_TEST_ROOT" "$tmp/release/deploy.log"
    mkdir -p "$DEPLOY_TEST_ROOT/etc" "$DEPLOY_TEST_ROOT/bin"
    [[ "$mode" == missing_logs ]] || mkdir "$DEPLOY_TEST_ROOT/logs"
    for file in "${files[@]}"; do printf 'old file\n' > "$DEPLOY_TEST_ROOT/$file"; done
    printf 'POSTGRES_PASSWORD=test\n' > "$DEPLOY_TEST_ROOT/etc/hw-bot.env"
    printf '[app]\n' > "$DEPLOY_TEST_ROOT/etc/production.toml"
    printf 'running\n' > "$DEPLOY_TEST_ROOT/state"
    printf 'example/hw-bot:previous\n' > "$DEPLOY_TEST_ROOT/image"
    printf 'previous release\n' > "$DEPLOY_TEST_ROOT/current-release.json"
    : > "$DEPLOY_TEST_LOG"
    status=0
    if [[ "$mode" == timeout_* ]]; then
        # Staging must finish on a busy runner before the deliberately blocked Docker call times out.
        timeout --signal=TERM --kill-after=5s 10s bash "$root/dist/bin/deploy.sh" \
            "$DEPLOY_TEST_ROOT" "$tmp/release" > "$tmp/output.log" 2>&1 || status=$?
        [[ "$status" == 124 ]]
    else
        bash "$root/dist/bin/deploy.sh" "$DEPLOY_TEST_ROOT" "$tmp/release" > "$tmp/output.log" 2>&1 || status=$?
    fi
    case "$mode" in
        missing_logs)
            [[ "$status" != 0 && "$(cat "$DEPLOY_TEST_ROOT/state")" == running ]]
            [[ ! -s "$DEPLOY_TEST_LOG" ]]
            ;;
        write_failure)
            [[ "$status" == 44 && "$(cat "$DEPLOY_TEST_ROOT/state")" == running ]]
            if grep -Eq '^(stop|backup)' "$DEPLOY_TEST_LOG"; then exit 1; fi
            ;;
        backup_failure|*rename_failure|hup_stop|term_stop|timeout_stop)
            [[ "$status" != 0 && "$(cat "$DEPLOY_TEST_ROOT/state")" == running ]]
            grep -qx 'start old-container' "$DEPLOY_TEST_LOG"
            ;;
        startup_failure|term_start|timeout_start|record_failure)
            [[ "$status" != 0 ]]
            if grep -q '^start ' "$DEPLOY_TEST_LOG"; then exit 1; fi
            ;;
        success)
            [[ "$status" == 0 && "$(cat "$DEPLOY_TEST_ROOT/state")" == new ]]
            jq -e --arg image "$image" '.image == $image and .previous_image == "example/hw-bot:previous" and .backup == "/backups/before-deploy.dump"' \
                "$DEPLOY_TEST_ROOT/current-release.json" > /dev/null
            ;;
    esac
    [[ "$(cat "$DEPLOY_TEST_ROOT/etc/hw-bot.env")" == 'POSTGRES_PASSWORD=test' ]]
    case "$mode" in
        success|startup_failure|term_start|timeout_start|record_failure)
            [[ "$(cat "$DEPLOY_TEST_ROOT/Makefile")" == 'new file' ]]
            [[ "$(cat "$DEPLOY_TEST_ROOT/compose.yml")" == 'new file' ]]
            ;;
        late_rename_failure)
            [[ "$(cat "$DEPLOY_TEST_ROOT/Makefile")" == 'new file' ]]
            [[ "$(cat "$DEPLOY_TEST_ROOT/compose.yml")" == 'old file' ]]
            if grep -q '^up ' "$DEPLOY_TEST_LOG"; then exit 1; fi
            ;;
        *)
            [[ "$(cat "$DEPLOY_TEST_ROOT/Makefile")" == 'old file' ]]
            [[ "$(cat "$DEPLOY_TEST_ROOT/compose.yml")" == 'old file' ]]
            if grep -q '^up ' "$DEPLOY_TEST_LOG"; then exit 1; fi
            ;;
    esac
    if [[ "$mode" != success ]]; then
        [[ "$(cat "$DEPLOY_TEST_ROOT/current-release.json")" == 'previous release' ]]
    fi
    if [[ "$mode" != success && "$mode" != missing_logs ]]; then
        [[ -s "$tmp/release/deploy.log" ]]
        case "$mode" in
            write_failure) phase=preflight ;;
            backup_failure) phase=backup ;;
            *rename_failure) phase=replace ;;
            *_stop) phase=stop ;;
            startup_failure|*_start) phase=start ;;
            record_failure) phase=record ;;
        esac
        grep -qx "Phase: $phase" "$tmp/release/deploy.log"
        [[ ! -e "$DEPLOY_TEST_ROOT/current-release.json.next" ]]
    fi
    for staged in "$DEPLOY_TEST_ROOT/".deploy.??????; do [[ ! -e "$staged" ]]; done
    flock -n "$DEPLOY_TEST_ROOT/.deploy.lock" true

    if [[ "$mode" != success ]]; then
        printf 'Deployment can be retried after %s\n' "$mode"
        mkdir -p "$DEPLOY_TEST_ROOT/logs"
        DEPLOY_TEST_MODE=success bash "$root/dist/bin/deploy.sh" "$DEPLOY_TEST_ROOT" "$tmp/release" > "$tmp/output.log" 2>&1
        [[ "$(cat "$DEPLOY_TEST_ROOT/state")" == new ]]
        [[ "$(cat "$DEPLOY_TEST_ROOT/compose.yml")" == 'new file' ]]
        jq -e --arg image "$image" '.image == $image and .backup == "/backups/before-deploy.dump"' \
            "$DEPLOY_TEST_ROOT/current-release.json" > /dev/null
    fi
done
