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
export DEPLOY_TEST_INSTALL DEPLOY_TEST_MV
DEPLOY_TEST_INSTALL=$(command -v install)
DEPLOY_TEST_MV=$(command -v mv)
mkdir -p "$tmp/bin" "$tmp/release/tools/deploy" "$tmp/release/tools/ops"
image="example/hw-bot@sha256:$(printf '%064d' 1)"
jq -n --arg image "$image" '{image: $image}' > "$tmp/release/release.json"
files=(tools/deploy/compose.yml tools/deploy/compose.bootstrap.yml tools/ops/backup.sh tools/ops/deploy.sh)
for file in "${files[@]}"; do printf 'new file\n' > "$tmp/release/$file"; done
cat > "$tmp/release/tools/ops/backup.sh" <<'EOF'
#!/bin/sh
printf 'backup\n' >> "$DEPLOY_TEST_LOG"
[ "$DEPLOY_TEST_MODE" != backup_failure ] || exit 42
printf '/backups/before-deploy.dump\n'
EOF
cat > "$tmp/bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == compose ]]; then shift 5; fi
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
cat > "$tmp/bin/install" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$DEPLOY_TEST_MODE" == write_failure && "$5" == */compose.yml ]]; then
    mkdir -p "$(dirname "$5")"
    : > "$5"
    exit 44
fi
exec "$DEPLOY_TEST_INSTALL" "$@"
EOF
cat > "$tmp/bin/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$DEPLOY_TEST_MODE:$2" in
    "rename_failure:$DEPLOY_TEST_ROOT/tools/deploy/compose.yml"|\
    "late_rename_failure:$DEPLOY_TEST_ROOT/tools/ops/deploy.sh"|\
    "env_rename_failure:$DEPLOY_TEST_ROOT/tools/deploy/.env.production"|\
    "record_failure:$DEPLOY_TEST_ROOT/current-release.json") exit 45 ;;
esac
exec "$DEPLOY_TEST_MV" "$@"
EOF
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH"

for mode in write_failure backup_failure rename_failure late_rename_failure env_rename_failure \
    hup_stop term_stop timeout_stop startup_failure term_start timeout_start record_failure success; do
    printf 'Deployment handles %s\n' "$mode"
    export DEPLOY_TEST_MODE="$mode"
    mkdir -p "$DEPLOY_TEST_ROOT/tools/deploy" "$DEPLOY_TEST_ROOT/tools/ops"
    for file in "${files[@]}"; do printf 'old file\n' > "$DEPLOY_TEST_ROOT/$file"; done
    printf 'HW_BOT_IMAGE=example/hw-bot:previous\n' > "$DEPLOY_TEST_ROOT/tools/deploy/.env.production"
    printf '[app]\n' > "$DEPLOY_TEST_ROOT/tools/deploy/production.toml"
    printf 'running\n' > "$DEPLOY_TEST_ROOT/state"
    printf 'example/hw-bot:previous\n' > "$DEPLOY_TEST_ROOT/image"
    printf 'previous release\n' > "$DEPLOY_TEST_ROOT/current-release.json"
    : > "$DEPLOY_TEST_LOG"
    status=0
    if [[ "$mode" == timeout_* ]]; then
        # Staging must finish on a busy runner before the deliberately blocked Docker call times out.
        timeout --signal=TERM --kill-after=5s 10s bash "$root/tools/ops/deploy.sh" \
            "$DEPLOY_TEST_ROOT" "$tmp/release" > "$tmp/output.log" 2>&1 || status=$?
        [[ "$status" == 124 ]]
    else
        bash "$root/tools/ops/deploy.sh" "$DEPLOY_TEST_ROOT" "$tmp/release" > "$tmp/output.log" 2>&1 || status=$?
    fi
    case "$mode" in
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
    if [[ "$mode" != success ]]; then
        [[ "$(cat "$DEPLOY_TEST_ROOT/current-release.json")" == 'previous release' ]]
    fi
    if [[ "$mode" != success ]]; then
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
    if [[ "$mode" == late_rename_failure || "$mode" == env_rename_failure ]]; then
        [[ "$(cat "$DEPLOY_TEST_ROOT/tools/deploy/compose.yml")" == 'new file' ]]
        [[ "$(cat "$DEPLOY_TEST_ROOT/tools/deploy/.env.production")" == 'HW_BOT_IMAGE=example/hw-bot:previous' ]]
        if grep -q '^up ' "$DEPLOY_TEST_LOG"; then exit 1; fi
    elif [[ "$mode" != success && "$mode" != startup_failure && "$mode" != term_start && "$mode" != timeout_start && "$mode" != record_failure ]]; then
        [[ "$(cat "$DEPLOY_TEST_ROOT/tools/deploy/compose.yml")" == 'old file' ]]
        [[ "$(cat "$DEPLOY_TEST_ROOT/tools/deploy/.env.production")" == 'HW_BOT_IMAGE=example/hw-bot:previous' ]]
        if grep -q '^up ' "$DEPLOY_TEST_LOG"; then exit 1; fi
    fi
    for staged in "$DEPLOY_TEST_ROOT/".deploy.??????; do [[ ! -e "$staged" ]]; done
    flock -n "$DEPLOY_TEST_ROOT/.deploy.lock" true

    if [[ "$mode" != success ]]; then
        printf 'Deployment can be retried after %s\n' "$mode"
        DEPLOY_TEST_MODE=success bash "$root/tools/ops/deploy.sh" "$DEPLOY_TEST_ROOT" "$tmp/release" > "$tmp/output.log" 2>&1
        [[ "$(cat "$DEPLOY_TEST_ROOT/state")" == new ]]
        jq -e --arg image "$image" '.image == $image and .backup == "/backups/before-deploy.dump"' \
            "$DEPLOY_TEST_ROOT/current-release.json" > /dev/null
    fi
done
