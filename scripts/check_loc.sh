#!/usr/bin/env bash
set -euo pipefail

HARD_LIMIT=500
SOFT_LIMIT=300
RATCHET="scripts/loc_violations.txt"
BASE_REF="${LOC_CHECK_BASE_REF:-origin/master}"
BASE_COMMIT="${LOC_CHECK_BASE_COMMIT:-}"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

FILES="$TMPDIR/files"
HARD="$TMPDIR/hard"
SOFT="$TMPDIR/soft"
CHANGED="$TMPDIR/changed"
SOFT_DRIFT="$TMPDIR/soft_drift"

: > "$HARD"
: > "$SOFT"
: > "$CHANGED"
: > "$SOFT_DRIFT"

line_count() {
    wc -l < "$1" | tr -d ' '
}

base_line_count() {
    if [ -n "$BASE_COMMIT" ] && git cat-file -e "$BASE_COMMIT:$1" 2>/dev/null; then
        git show "$BASE_COMMIT:$1" | wc -l | tr -d ' '
    else
        echo 0
    fi
}

git ls-files '*.rs' | grep -E '^(src|tests)/' > "$FILES" || true

if [ -z "$BASE_COMMIT" ] && git rev-parse --verify "$BASE_REF" >/dev/null 2>&1; then
    BASE_COMMIT=$(git merge-base "$BASE_REF" HEAD)
fi

while IFS= read -r f; do
    [ -n "$f" ] || continue
    lc=$(line_count "$f")
    if [ "$lc" -gt "$HARD_LIMIT" ]; then
        echo "$lc $f" >> "$HARD"
    elif [ "$lc" -gt "$SOFT_LIMIT" ]; then
        echo "$lc $f" >> "$SOFT"
    fi
done < "$FILES"

if [ -n "$BASE_COMMIT" ]; then
    {
        git diff --name-only --diff-filter=ACMRT "$BASE_COMMIT"...HEAD -- '*.rs'
        git diff --name-only --diff-filter=ACMRT HEAD -- '*.rs'
    } | grep -E '^(src|tests)/' | sort -u > "$CHANGED" || true

    while IFS= read -r f; do
        [ -n "$f" ] || continue
        [ -f "$f" ] || continue
        lc=$(line_count "$f")
        [ "$lc" -gt "$SOFT_LIMIT" ] || continue

        old_lc=$(base_line_count "$f")
        if [ "$old_lc" -le "$SOFT_LIMIT" ] || [ "$lc" -gt "$old_lc" ]; then
            echo "$lc $old_lc $f" >> "$SOFT_DRIFT"
        fi
    done < "$CHANGED"
fi

if [ -s "$SOFT" ]; then
    echo "loc-check: soft-limit watchlist (> $SOFT_LIMIT and <= $HARD_LIMIT LOC):"
    sort -nr "$SOFT" | sed 's/^/  /'
else
    echo "loc-check: no files over soft limit ($SOFT_LIMIT LOC)"
fi

if [ ! -s "$HARD" ]; then
    echo "loc-check: all files under hard limit ($HARD_LIMIT LOC)"
else
    NEW_HARD="$TMPDIR/new_hard"
    : > "$NEW_HARD"
    while read -r lc f; do
        if ! grep -qx "$f" "$RATCHET" 2>/dev/null; then
            echo "$lc $f" >> "$NEW_HARD"
        fi
    done < "$HARD"

    if [ -s "$NEW_HARD" ]; then
        echo "loc-check: NEW hard violations (not in ratchet):"
        sort -nr "$NEW_HARD" | sed 's/^/  /'
        echo ""
        echo "Reduce file size to <= $HARD_LIMIT LOC or add an intentional exemption to $RATCHET"
        exit 1
    fi

    count=$(wc -l < "$HARD" | tr -d ' ')
    echo "loc-check: $count known hard violations (in ratchet), no new hard violations"
fi

if [ -z "$BASE_COMMIT" ]; then
    echo "loc-check: soft-limit drift ratchet skipped; no baseline ref '$BASE_REF'"
elif [ -s "$SOFT_DRIFT" ]; then
    echo "loc-check: soft-limit drift violations:"
    while read -r lc old_lc f; do
        echo "  $f:$lc (baseline $old_lc)"
    done < "$SOFT_DRIFT"
    echo ""
    echo "Keep changed files <= $SOFT_LIMIT LOC, avoid growing existing soft-limit files, or split by domain boundary."
    exit 1
else
    echo "loc-check: no new soft-limit drift against $BASE_COMMIT"
fi
