#!/usr/bin/env bash
set -euo pipefail

version=${1:?usage: bash tools/release.sh X.Y.Z}
tag=v${version#v}

[[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || {
    printf 'Release tag must be vX.Y.Z: %s\n' "$tag" >&2
    exit 1
}
[[ "$(git branch --show-current)" == master ]] || { printf '%s\n' 'Release from master' >&2; exit 1; }
[[ -z "$(git status --porcelain)" ]] || { printf '%s\n' 'Working tree is not clean' >&2; exit 1; }

git push --atomic origin master "HEAD:refs/tags/$tag"
git tag "$tag"
printf 'Pushed %s at %s\n' "$tag" "$(git rev-parse --short HEAD)"
