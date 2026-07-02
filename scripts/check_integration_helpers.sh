#!/usr/bin/env bash
set -euo pipefail

matches="$(
    find tests -name '*.rs' -type f -print0 \
        | xargs -0 awk '
            FNR == 1 { prev = "" }
            /^[[:space:]]*mod common[[:space:]]*;/ &&
                prev !~ /^[[:space:]]*#\[path = "common\/mod\.rs"\][[:space:]]*$/ {
                print FILENAME ":" FNR ":" $0
            }
            { prev = $0 }
        '
)"

if [ -n "$matches" ]; then
    echo "helper-import-check: bare integration helper imports found:"
    echo "$matches" | sed 's/^/  /'
    echo ""
    echo "Use an explicit #[path = \"common/mod.rs\"] annotation instead."
    exit 1
fi

echo "helper-import-check: integration helper imports are explicit"
