#!/bin/sh
set -eu
umask 077

image=${1:?usage: sh tools/actions/smoke.sh IMAGE}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/hw-bot-smoke.XXXXXX")
diagnostics=${2:-"$tmp/diagnostics"}
installation="$tmp/installation"
export COMPOSE_PROJECT_NAME="hw-bot-smoke-$$"
compose() {
    sh "$installation/bin/compose.sh" "$@"
}
cleanup() {
    status=$?
    set +e
    export HW_BOT_IMAGE="$image"
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

sh tools/actions/package.sh HEAD "$tmp/deploy.tar.gz"
mkdir -p "$installation/etc" "$installation/logs"
tar -xzf "$tmp/deploy.tar.gz" -C "$installation" --strip-components=1
sed '/^\[server\]$/,/^\[/ {
    s/^host = .*/host = "::1"/
    s/^port = .*/port = 3001/
}' "$installation/examples/production.toml" > "$installation/etc/production.toml"
grep -qx 'host = "::1"' "$installation/etc/production.toml"
grep -qx 'port = 3001' "$installation/etc/production.toml"
cp "$installation/examples/bootstrap.json" "$installation/etc/bootstrap.json"
chmod 0770 "$installation/logs"
sudo chgrp 10001 "$installation/etc/production.toml" "$installation/etc/bootstrap.json" "$installation/logs"
cat > "$installation/etc/hw-bot.env" <<EOF
POSTGRES_PASSWORD=smoke
APP__TELEGRAM__ENABLED=false
APP__NOTIFICATION_WORKER__ENABLED=false
APP__GROUP_INVITES__INVITE_SIGNING_SECRET=AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8
EOF
jq -n --arg image "$image" '{version: "0.0.0", commit: "smoke", image: $image, deployed_at: "smoke"}' \
    > "$installation/current-release.json"

make -C "$installation" bootstrap
make -C "$installation" bootstrap
ls "$installation"/logs/hw-bot.*.log > /dev/null

make -C "$installation" backup
backup=$(cat "$installation/backups/.last-success")
make -C "$installation" restore DUMP="$backup" CONFIRM=yes
make -C "$installation" status
make -C "$installation" queues

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
