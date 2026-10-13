#!/bin/sh
set -eu

dist=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
root=${HW_BOT_ROOT:-"$dist"}
if [ -z "${HW_BOT_IMAGE:-}" ]; then
    [ -f "$root/current-release.json" ] || {
        printf '%s\n' "Missing $root/current-release.json: deploy a release or set HW_BOT_IMAGE" >&2
        exit 1
    }
    HW_BOT_IMAGE=$(jq -er '.image' "$root/current-release.json")
    export HW_BOT_IMAGE
fi
exec docker compose --project-directory "$root" --env-file "$root/etc/hw-bot.env" \
    -f "$dist/compose.yml" "$@"
