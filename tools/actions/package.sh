#!/bin/sh
set -eu

commit=${1:?usage: sh tools/actions/package.sh COMMIT OUTPUT}
output=${2:?usage: sh tools/actions/package.sh COMMIT OUTPUT}

git archive --format=tar.gz --output="$output" "$commit" \
    .github/README.md tools/deploy/compose.yml tools/deploy/compose.bootstrap.yml \
    tools/ops/backup.sh tools/ops/deploy.sh \
    tools/deploy/.env.example tools/deploy/production.example.toml \
    tools/deploy/bootstrap.example.json
