#!/usr/bin/env bash
set -euo pipefail

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

sh tools/actions/package.sh "$COMMIT" deploy.tar.gz
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

release_state=$(gh api --paginate "repos/$GH_REPO/releases?per_page=100" \
    | jq -r --arg tag "$TAG" '.[] | select(.tag_name == $tag) | .draft')
if [[ -z "$release_state" ]]; then
    gh release create "$TAG" release.json deploy.tar.gz --verify-tag --generate-notes --title "$TAG" --draft
    gh release edit "$TAG" --draft=false
elif [[ "$release_state" == true ]]; then
    gh release upload "$TAG" release.json deploy.tar.gz --clobber
    gh release edit "$TAG" --draft=false
else
    [[ "$release_state" == false ]]
    mkdir "$tmp/existing-release"
    gh release download "$TAG" --pattern release.json --pattern deploy.tar.gz --dir "$tmp/existing-release"
    cmp release.json "$tmp/existing-release/release.json"
    cmp deploy.tar.gz "$tmp/existing-release/deploy.tar.gz"
fi
printf '### Release %s\n\nImage: %s\n\nCommit: %s\n\nBranch: release/%s\n' \
    "$TAG" "$IMAGE" "$COMMIT" "$VERSION" >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"
