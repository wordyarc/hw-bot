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
files=(Makefile tools/deploy/compose.yml tools/deploy/compose.bootstrap.yml tools/ops/backup.sh tools/ops/deploy.sh)
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
    inspect) printf 'example/hw-bot:previous\n' ;;
    config|pull|logs) ;;
    stop) printf 'stopped\n' > "$DEPLOY_TEST_ROOT/state" ;;
    start)
        [[ "$2" == old-container ]]
        printf 'running\n' > "$DEPLOY_TEST_ROOT/state"
        ;;
    up)
        [[ "$DEPLOY_TEST_MODE" != startup_failure ]] || exit 43
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
if [[ "$DEPLOY_TEST_MODE" == rename_failure && "$2" == "$DEPLOY_TEST_ROOT/tools/deploy/compose.yml" ]]; then
    exit 45
fi
exec "$DEPLOY_TEST_MV" "$@"
EOF
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH"

for mode in write_failure backup_failure rename_failure startup_failure success; do
    printf 'Deployment handles %s\n' "$mode"
    export DEPLOY_TEST_MODE="$mode"
    mkdir -p "$DEPLOY_TEST_ROOT/tools/deploy" "$DEPLOY_TEST_ROOT/tools/ops"
    for file in "${files[@]}"; do printf 'old file\n' > "$DEPLOY_TEST_ROOT/$file"; done
    printf 'HW_BOT_IMAGE=example/hw-bot:previous\n' > "$DEPLOY_TEST_ROOT/tools/deploy/.env.production"
    printf '[app]\n' > "$DEPLOY_TEST_ROOT/tools/deploy/production.toml"
    printf 'running\n' > "$DEPLOY_TEST_ROOT/state"
    printf 'previous release\n' > "$DEPLOY_TEST_ROOT/current-release.json"
    : > "$DEPLOY_TEST_LOG"
    status=0
    bash "$root/tools/ops/deploy.sh" "$DEPLOY_TEST_ROOT" "$tmp/release" > "$tmp/output.log" 2>&1 || status=$?
    case "$mode" in
        write_failure)
            [[ "$status" == 44 && "$(cat "$DEPLOY_TEST_ROOT/state")" == running ]]
            if grep -Eq '^(stop|backup)' "$DEPLOY_TEST_LOG"; then exit 1; fi
            ;;
        backup_failure|rename_failure)
            [[ "$status" != 0 && "$(cat "$DEPLOY_TEST_ROOT/state")" == running ]]
            grep -qx 'start old-container' "$DEPLOY_TEST_LOG"
            ;;
        startup_failure)
            [[ "$status" == 43 ]]
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
    if [[ "$mode" != success && "$mode" != startup_failure ]]; then
        [[ "$(cat "$DEPLOY_TEST_ROOT/tools/deploy/compose.yml")" == 'old file' ]]
        [[ "$(cat "$DEPLOY_TEST_ROOT/tools/deploy/.env.production")" == 'HW_BOT_IMAGE=example/hw-bot:previous' ]]
        if grep -q '^up ' "$DEPLOY_TEST_LOG"; then exit 1; fi
    fi
    for staged in "$DEPLOY_TEST_ROOT/".deploy.??????; do [[ ! -e "$staged" ]]; done
done
