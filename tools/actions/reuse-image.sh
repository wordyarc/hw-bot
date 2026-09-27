#!/usr/bin/env bash
set -euo pipefail

response=$(mktemp)
trap 'rm -f "$response"' EXIT

token=$(curl --fail --silent --show-error --connect-timeout 10 --max-time 30 \
    --user "$DOCKERHUB_USERNAME:$DOCKERHUB_TOKEN" --get \
    --data-urlencode service=registry.docker.io --data-urlencode "scope=repository:$IMAGE:pull" \
    https://auth.docker.io/token | jq -er '.token')
printf '::add-mask::%s\n' "$token"
status=$(curl --silent --show-error --connect-timeout 10 --max-time 30 \
    --header "Authorization: Bearer $token" \
    --header 'Accept: application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json' \
    --output "$response" --write-out '%{http_code}' \
    "https://registry-1.docker.io/v2/$IMAGE/manifests/$VERSION")

case "$status" in
    200)
        docker pull "$IMAGE:$VERSION"
        revision=$(docker image inspect "$IMAGE:$VERSION" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')
        published_version=$(docker image inspect "$IMAGE:$VERSION" --format '{{index .Config.Labels "org.opencontainers.image.version"}}')
        [[ "$revision" == "$COMMIT" && "$published_version" == "$VERSION" ]]
        printf 'found=true\n' >> "$GITHUB_OUTPUT"
        ;;
    404)
        jq -e 'any(.errors[]; .code == "MANIFEST_UNKNOWN")' "$response" > /dev/null
        printf 'found=false\n' >> "$GITHUB_OUTPUT"
        ;;
    *)
        printf 'Registry lookup failed: HTTP %s\n' "$status" >&2
        cat "$response" >&2
        exit 1
        ;;
esac
