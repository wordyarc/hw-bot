#!/bin/sh
set -eu

cd "$(dirname -- "$0")/.."

jq -r '"release  \(.version), deployed \(.deployed_at)"' current-release.json
recorded=$(jq -r '.image' current-release.json)
container=$(sh bin/compose.sh ps --all -q app)
running=none
if [ -n "$container" ]; then
    running=$(docker inspect --format '{{.Config.Image}}' "$container")
fi
if [ "$running" = "$recorded" ]; then
    printf 'image    %s\n' "$recorded"
else
    printf 'image    running %s, recorded %s\n' "$running" "$recorded"
    printf '%s\n' 'WARNING  the running image is not the last successfully deployed one'
fi

if [ -f backups/.last-success ]; then
    dump=$(cat backups/.last-success)
    if [ -f "$dump" ]; then
        hours=$(( ($(date +%s) - $(stat -c %Y "$dump")) / 3600 ))
        printf 'backup   %s, %s h ago\n' "$dump" "$hours"
    else
        printf 'backup   %s, missing\n' "$dump"
        printf '%s\n' 'WARNING  the last successful backup is gone'
    fi
else
    printf '%s\n' 'backup   none'
fi
printf '\n'
sh bin/compose.sh ps --all
printf '\n'
df -h .
