#!/usr/bin/env bash
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel)
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

write_lines() {
    awk -v n="$1" 'BEGIN { for (i = 1; i <= n; i++) print "// line" }' > "$2"
}

run_loc_check() {
    LOC_CHECK_BASE_COMMIT="$BASE_COMMIT" bash scripts/check_loc.sh
}

expect_success() {
    name="$1"
    if ! output=$(run_loc_check 2>&1); then
        printf 'FAIL: %s\n%s\n' "$name" "$output"
        exit 1
    fi
    printf 'ok: %s\n' "$name"
}

expect_failure() {
    name="$1"
    expected="$2"
    if output=$(run_loc_check 2>&1); then
        printf 'FAIL: %s unexpectedly passed\n%s\n' "$name" "$output"
        exit 1
    fi
    if ! printf '%s\n' "$output" | grep -q "$expected"; then
        printf 'FAIL: %s missing expected output %s\n%s\n' "$name" "$expected" "$output"
        exit 1
    fi
    printf 'ok: %s\n' "$name"
}

cd "$TMPDIR"
git init -q
git config user.email loc-check@example.invalid
git config user.name 'LOC Check Test'

mkdir -p scripts src tests
cp "$ROOT/scripts/check_loc.sh" scripts/check_loc.sh
: > scripts/loc_violations.txt

write_lines 301 src/legacy.rs
git add .
git commit -qm baseline
BASE_COMMIT=$(git rev-parse HEAD)

expect_success "unchanged baseline soft-limit file is reported but allowed"

write_lines 302 src/legacy.rs
expect_failure "growing an existing soft-limit file fails" "src/legacy.rs:302 (baseline 301)"

git checkout -q -- src/legacy.rs
write_lines 301 tests/new_case.rs
git add tests/new_case.rs
expect_failure "new tracked soft-limit file fails" "tests/new_case.rs:301 (baseline 0)"

git rm -fq tests/new_case.rs
write_lines 300 src/legacy.rs
expect_success "shrinking a soft-limit file to target passes"

write_lines 501 src/huge.rs
git add src/huge.rs
expect_failure "new hard-limit file fails" "src/huge.rs"
