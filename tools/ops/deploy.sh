#!/usr/bin/env bash
set -euo pipefail
umask 077

root=${1:?usage: bash tools/ops/deploy.sh ROOT RELEASE_DIRECTORY}
release_dir=${2:?usage: bash tools/ops/deploy.sh ROOT RELEASE_DIRECTORY}
root=$(cd "$root" && pwd)
release_dir=$(cd "$release_dir" && pwd)
cd "$root"

exec 9> "$root/.deploy.lock"
flock -n 9 || { printf '%s\n' 'Another deployment is running' >&2; exit 1; }

export HW_BOT_ENV_FILE="$root/tools/deploy/.env.production"
export HW_BOT_CONFIG_DIR="$root/tools/deploy"
export BACKUP_DIR="$root/backups"
image=$(jq -er '.image' "$release_dir/release.json")
[[ "$image" =~ ^[a-zA-Z0-9][^[:space:]]*@sha256:[0-9a-f]{64}$ ]] || {
    printf '%s\n' 'Release image must be pinned to a SHA-256 digest' >&2
    exit 1
}
for file in "$HW_BOT_ENV_FILE" "$HW_BOT_CONFIG_DIR/production.toml"; do
    [[ -f "$file" && -r "$file" ]] || { printf 'Missing or unreadable file: %s\n' "$file" >&2; exit 1; }
done
files=(tools/deploy/compose.yml tools/deploy/compose.bootstrap.yml
    tools/ops/backup.sh tools/ops/deploy.sh)
for file in "${files[@]}"; do
    [[ -f "$release_dir/$file" ]] || { printf 'Missing release file: %s\n' "$file" >&2; exit 1; }
done

compose() {
    docker compose --env-file "$HW_BOT_ENV_FILE" -f tools/deploy/compose.yml "$@"
}
resume_old=false
stage=
phase=preflight
cleanup() {
    status=$?
    set +e
    if [[ -n "$stage" ]]; then
        rm -rf "$stage"
    fi
    rm -f "$root/current-release.json.next"
    if (( status != 0 )); then
        if [[ "$resume_old" == true ]]; then
            docker start "$container" || true
        fi
        printf 'Phase: %s\nExit code: %s\nImage: %s\nPrevious image: %s\nBackup: %s\n' \
            "$phase" "$status" "$image" "${previous_image:-unknown}" "${backup:-none}" > "$release_dir/deploy.log"
        compose ps --all >> "$release_dir/deploy.log" 2>&1 || true
        compose logs --no-color --tail 100 app database >> "$release_dir/deploy.log" 2>&1 || true
        cat "$release_dir/deploy.log" >&2
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

container=$(compose ps --all -q app)
[[ -n "$container" ]] || { printf '%s\n' 'Bootstrap the installation before deploying' >&2; exit 1; }
previous_image=$(docker inspect --format '{{.Config.Image}}' "$container")
HW_BOT_IMAGE="$image" docker compose --env-file "$HW_BOT_ENV_FILE" \
    -f "$release_dir/tools/deploy/compose.yml" config --quiet
docker pull "$image"

stage=$(mktemp -d "$root/.deploy.XXXXXX")
for file in "${files[@]}"; do
    install -D -m 0644 "$release_dir/$file" "$stage/$file"
done
awk -v image="$image" '
    /^HW_BOT_IMAGE=/ { if (!written) print "HW_BOT_IMAGE=" image; written = 1; next }
    { print }
    END { if (!written) print "HW_BOT_IMAGE=" image }
' "$HW_BOT_ENV_FILE" > "$stage/.env.production"

phase=stop
resume_old=true
compose stop app
phase=backup
backup=$(sh "$release_dir/tools/ops/backup.sh")

phase=replace
for file in "${files[@]}"; do
    mv "$stage/$file" "$root/$file"
done
mv "$stage/.env.production" "$HW_BOT_ENV_FILE"
export HW_BOT_IMAGE="$image"
phase=start
resume_old=false
compose up -d --no-deps --force-recreate --wait --wait-timeout 120 app

phase=record
jq --arg previous_image "$previous_image" --arg backup "$backup" \
    --arg deployed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '. + {previous_image: $previous_image, backup: $backup, deployed_at: $deployed_at}' \
    "$release_dir/release.json" > "$root/current-release.json.next"
mv "$root/current-release.json.next" "$root/current-release.json"
printf 'Deployed %s; previous image: %s; backup: %s\n' "$image" "$previous_image" "$backup"
