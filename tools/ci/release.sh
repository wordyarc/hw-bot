#!/usr/bin/env bash
set -euo pipefail

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

git archive --format=tar.gz --output=deploy.tar.gz "$COMMIT" \
    Makefile tools/deploy/compose.yml tools/deploy/compose.bootstrap.yml \
    tools/deploy/backup.sh tools/deploy/deploy.sh \
    tools/deploy/.env.example tools/deploy/production.example.toml \
    tools/deploy/bootstrap.example.json
checksum=$(sha256sum deploy.tar.gz | cut -d ' ' -f 1)
jq -n --arg version "$VERSION" --arg commit "$COMMIT" \
    --arg image "$IMAGE" --arg checksum "$checksum" \
    '{version: $version, commit: $commit, image: $image, archive_sha256: $checksum}' > release.json

branch="release/$VERSION"
if existing=$(git rev-parse --verify --quiet "refs/remotes/origin/$branch"); then
    [[ "$existing" == "$COMMIT" ]]
else
    gh api "repos/$GH_REPO/git/refs" -f ref="refs/heads/$branch" -f sha="$COMMIT"
fi

if gh release view "$TAG" --json isDraft > "$tmp/release-state.json" 2> "$tmp/release-error.log"; then
    if jq -e '.isDraft' "$tmp/release-state.json" > /dev/null; then
        gh release upload "$TAG" release.json deploy.tar.gz --clobber
        gh release edit "$TAG" --draft=false
    else
        mkdir "$tmp/existing-release"
        gh release download "$TAG" --pattern release.json --pattern deploy.tar.gz --dir "$tmp/existing-release"
        cmp release.json "$tmp/existing-release/release.json"
        cmp deploy.tar.gz "$tmp/existing-release/deploy.tar.gz"
    fi
else
    grep -q 'release not found' "$tmp/release-error.log" || { cat "$tmp/release-error.log"; exit 1; }
    gh release create "$TAG" release.json deploy.tar.gz --verify-tag --generate-notes --title "$TAG" --draft
    gh release edit "$TAG" --draft=false
fi
printf '### Release %s\n\nImage: %s\n\nCommit: %s\n\nBranch: release/%s\n' \
    "$TAG" "$IMAGE" "$COMMIT" "$VERSION" >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"
