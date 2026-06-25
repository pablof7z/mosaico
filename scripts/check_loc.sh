#!/usr/bin/env bash
set -euo pipefail

LIMIT=500
RATCHET="scripts/loc_violations.txt"
VIOLATIONS=()

while IFS= read -r f; do
    lc=$(wc -l < "$f" | tr -d ' ')
    if [ "$lc" -gt "$LIMIT" ]; then
        VIOLATIONS+=("$f:$lc")
    fi
done < <(find src -name '*.rs' -not -path '*/target/*')

if [ ${#VIOLATIONS[@]} -eq 0 ]; then
    echo "loc-check: all files under $LIMIT LOC"
    exit 0
fi

NEW_VIOLATIONS=()
for v in "${VIOLATIONS[@]}"; do
    f="${v%%:*}"
    if ! grep -qx "$f" "$RATCHET" 2>/dev/null; then
        NEW_VIOLATIONS+=("$v")
    fi
done

if [ ${#NEW_VIOLATIONS[@]} -gt 0 ]; then
    echo "loc-check: NEW violations (not in ratchet):"
    for v in "${NEW_VIOLATIONS[@]}"; do
        echo "  $v"
    done
    echo ""
    echo "Either reduce file size to <= $LIMIT LOC or add to $RATCHET"
    exit 1
fi

echo "loc-check: ${#VIOLATIONS[@]} known violations (in ratchet), no new violations"