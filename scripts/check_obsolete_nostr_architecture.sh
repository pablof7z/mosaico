#!/usr/bin/env bash
set -euo pipefail

# The direct SDK cutover has no compatibility layer. Keep this ratchet separate
# from formatting so a retired engine or bridge cannot return unnoticed.
retired_runtime='fava::|fava_host|fava_events|fava_simple_groups|fava_subscriptions|fava_publications|fava_relays|fava_accounts|fava_signer|NMP|Fava'
retired_bridge='PlainRelay|views_handle|ObservedViews|SubscriptionQuery|observe_sync|all_terminal[[:space:]]*\(|\.settled[[:space:]]*\('
absolute_dependency='path[[:space:]]*=[[:space:]]*"/'

matches="$(git grep -In -E "$retired_runtime|$retired_bridge" -- . ':!scripts/check_obsolete_nostr_architecture.sh' ':!openspec/changes/replace-fava-with-nostr-sdk/**' || true)"
absolute_matches="$(git grep -Iin -E "$absolute_dependency" -- '*.toml' || true)"
matches="${matches}${matches:+$'\n'}${absolute_matches}"

if [[ -n "$matches" ]]; then
    printf '%s\n' 'retired Nostr architecture or absolute dependency is forbidden'
    printf '%s\n' "$matches"
    exit 1
fi
