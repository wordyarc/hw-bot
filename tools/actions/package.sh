#!/bin/sh
set -eu

commit=${1:?usage: sh tools/actions/package.sh COMMIT OUTPUT}
output=${2:?usage: sh tools/actions/package.sh COMMIT OUTPUT}

git archive --format=tar.gz --output="$output" "$commit" dist
