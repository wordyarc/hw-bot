#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
cleanup() {
    status=$?
    if (( status != 0 )); then cat "$tmp/output.log" >&2 || true; fi
    rm -rf "$tmp"
}
trap cleanup EXIT

mkdir -p "$tmp/bin" "$tmp/work/tools/actions"
cp "$root/tools/actions/package.sh" "$tmp/work/tools/actions/package.sh"
export RELEASE_TEST_LOG="$tmp/calls.log"
export IMAGE=example/hw-bot VERSION=1.2.3 COMMIT=revision TAG=v1.2.3 GH_REPO=example/hw-bot
export DOCKERHUB_USERNAME=test DOCKERHUB_TOKEN=test GITHUB_OUTPUT="$tmp/result" GITHUB_STEP_SUMMARY="$tmp/summary"
cat > "$tmp/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${*: -1}" == https://auth.docker.io/token ]]; then
    [[ "$RELEASE_TEST_MODE" != auth_failure ]] || exit 22
    printf '{"token":"test-token"}\n'
    exit 0
fi
while [[ "$1" != --output ]]; do shift; done
case "$RELEASE_TEST_MODE" in
    missing) printf '{"errors":[{"code":"MANIFEST_UNKNOWN","message":"arbitrary wording"}]}' > "$2"; printf 404 ;;
    missing_repository) printf '{"errors":[{"code":"NAME_UNKNOWN"}]}' > "$2"; printf 404 ;;
    unauthorized) printf '{}' > "$2"; printf 401 ;;
    unavailable) printf '{}' > "$2"; printf 503 ;;
    network_failure) exit 7 ;;
    *) printf '{}' > "$2"; printf 200 ;;
esac
EOF
cat > "$tmp/bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$RELEASE_TEST_LOG"
if [[ "$1" == pull ]]; then
    [[ "$RELEASE_TEST_MODE" != pull_failure ]]
elif [[ "${*: -1}" == *revision* ]]; then
    if [[ "$RELEASE_TEST_MODE" == wrong_revision ]]; then printf 'different\n'; else printf '%s\n' "$COMMIT"; fi
else
    if [[ "$RELEASE_TEST_MODE" == wrong_version ]]; then printf 'different\n'; else printf '%s\n' "$VERSION"; fi
fi
EOF
cat > "$tmp/bin/git" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
    rev-parse) printf '%s\n' "$COMMIT" ;;
    archive)
        for arg in "$@"; do
            if [[ "$arg" == --output=* ]]; then printf 'archive\n' > "${arg#--output=}"; fi
        done
        ;;
    *) exit 99 ;;
esac
EOF
cat > "$tmp/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$RELEASE_TEST_LOG"
if [[ "$1" == api ]]; then
    [[ "$2" == --paginate ]]
    printf '[{"tag_name":"v0.0.1","draft":false}]\n'
    case "$RELEASE_TEST_MODE" in
        absent) printf '[]\n' ;;
        draft) printf '[{"tag_name":"v1.2.3","draft":true}]\n' ;;
        published|changed_asset) printf '[{"tag_name":"v1.2.3","draft":false}]\n' ;;
        api_failure) exit 1 ;;
        *) exit 99 ;;
    esac
elif [[ "$2" == download ]]; then
    while [[ "$1" != --dir ]]; do shift; done
    cp release.json deploy.tar.gz "$2/"
    if [[ "$RELEASE_TEST_MODE" == changed_asset ]]; then printf 'different\n' > "$2/deploy.tar.gz"; fi
fi
EOF
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH"

for mode in missing existing wrong_revision wrong_version pull_failure auth_failure missing_repository unauthorized unavailable network_failure; do
    printf 'Image lookup handles %s\n' "$mode"
    export RELEASE_TEST_MODE="$mode"
    : > "$GITHUB_OUTPUT"
    : > "$RELEASE_TEST_LOG"
    status=0
    bash "$root/tools/actions/reuse-image.sh" > "$tmp/output.log" 2>&1 || status=$?
    case "$mode" in
        missing)
            [[ "$status" == 0 && "$(cat "$GITHUB_OUTPUT")" == found=false ]]
            [[ ! -s "$RELEASE_TEST_LOG" ]]
            ;;
        existing)
            [[ "$status" == 0 && "$(cat "$GITHUB_OUTPUT")" == found=true ]]
            grep -qx 'pull example/hw-bot:1.2.3' "$RELEASE_TEST_LOG"
            ;;
        *) [[ "$status" != 0 && ! -s "$GITHUB_OUTPUT" ]] ;;
    esac
done

cd "$tmp/work"
for mode in absent draft published changed_asset api_failure; do
    printf 'Release publication handles %s\n' "$mode"
    export RELEASE_TEST_MODE="$mode"
    : > "$RELEASE_TEST_LOG"
    status=0
    bash "$root/tools/actions/release.sh" > "$tmp/output.log" 2>&1 || status=$?
    case "$mode" in
        absent) [[ "$status" == 0 ]]; grep -q '^release create ' "$RELEASE_TEST_LOG" ;;
        draft) [[ "$status" == 0 ]]; grep -q '^release upload ' "$RELEASE_TEST_LOG" ;;
        published) [[ "$status" == 0 ]]; grep -q '^release download ' "$RELEASE_TEST_LOG" ;;
        changed_asset|api_failure) [[ "$status" != 0 ]] ;;
    esac
    if [[ "$mode" != absent && "$mode" != draft ]]; then
        if grep -Eq '^release (create|upload|edit) ' "$RELEASE_TEST_LOG"; then exit 1; fi
    fi
done
