#!/bin/sh
set -eu

if [ ! -f /app/config/local.toml ] || [ ! -r /app/config/local.toml ]; then
    printf '%s\n' 'Production config /app/config/local.toml must be a readable file' >&2
    exit 1
fi

export APP__SERVER__HOST=127.0.0.1 APP__SERVER__PORT=3000
exec /usr/local/bin/hw-bot "$@"
