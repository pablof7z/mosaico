#!/usr/bin/env bash
# Delegate a full selected-instance state wipe to Mosaico's owned reset door.
# Usage: ./scripts/reset.sh --yes-i-know-this-wipes-local-state

set -euo pipefail

case "${1:-}" in
  --yes-i-know-this-wipes-local-state)
    ;;
  *)
    cat >&2 <<EOF
Refusing to reset without explicit confirmation.

Options:
  $0 --yes-i-know-this-wipes-local-state
      Wipe selected local runtime state. Configuration and external relays stay.
EOF
    exit 2
    ;;
esac

if ! command -v mosaico >/dev/null 2>&1; then
  echo "mosaico is not on PATH; refusing to bypass its coordinated reset" >&2
  exit 1
fi
exec mosaico daemon reset-state --yes-i-know-this-wipes-local-state
