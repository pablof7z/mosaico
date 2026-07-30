#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

hosted="src/session_host/launch/hosted.rs"

assert_single_site() {
    local label=$1
    local pattern=$2
    local sites=()
    mapfile -t sites < <(
        grep -R -n -E --include='*.rs' "$pattern" src/session_host/launch || true
    )
    if [[ ${#sites[@]} -ne 1 || ${sites[0]%%:*} != "$hosted" ]]; then
        echo "hosted-open seam drift: expected one $label site in $hosted" >&2
        printf '  %s\n' "${sites[@]:-<none>}" >&2
        exit 1
    fi
}

assert_single_site "LaunchSpec construction" '(^|[^[:alnum:]_])LaunchSpec[[:space:]]*\{'
assert_single_site "hosted bootstrap" 'bootstrap_hosted_session_start[[:space:]]*\('
assert_single_site "fresh transport opening" 'transport\.launch[[:space:]]*\('
assert_single_site "resume transport opening" '\.resume[[:space:]]*\('
assert_single_site "endpoint rollback" 'kill_endpoint[[:space:]]*\(&transport'

echo "hosted-open-seam-check: ok"
