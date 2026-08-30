#!/usr/bin/env bash
set -euo pipefail

# The superseded three-letter fabric vocabulary must not return in either
# tracked paths or tracked text. Character classes keep the forbidden token
# out of the ratchet itself while still matching every case variant.
pattern='[Nn][Mm][Pp]'
path_matches="$(git ls-files | LC_ALL=C grep -E "$pattern" || true)"
content_matches="$(git grep -Iin -E "$pattern" -- . || true)"

if [[ -n "$path_matches" || -n "$content_matches" ]]; then
    printf '%s\n' 'superseded fabric vocabulary is forbidden'
    [[ -z "$path_matches" ]] || printf '%s\n' "$path_matches"
    [[ -z "$content_matches" ]] || printf '%s\n' "$content_matches"
    exit 1
fi
