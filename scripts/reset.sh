#!/usr/bin/env bash
# Wipe all runtime state (local + remote relay) without touching configs.
# Usage:
#   ./scripts/reset.sh --local-only
#   ./scripts/reset.sh --yes-i-know-this-wipes-the-relay

set -euo pipefail

RELAY_HOST="pablo@157.180.102.242"
EDGE_HOME="${TENEX_EDGE_HOME:-$HOME/.tenex-edge}"

LOCAL_ONLY=false
case "${1:-}" in
  --local-only)
    LOCAL_ONLY=true
    ;;
  --yes-i-know-this-wipes-the-relay)
    ;;
  *)
    cat >&2 <<EOF
Refusing to reset without explicit confirmation.

This deletes local runtime state under:
  $EDGE_HOME

Options:
  $0 --local-only
      Wipe local state only (db, sessions, sockets). Relay untouched.

  $0 --yes-i-know-this-wipes-the-relay
      Wipe local state AND SSHes to $RELAY_HOST to wipe nip29.f7z.io relay data.
EOF
    exit 2
    ;;
esac

if [[ ! -d "$EDGE_HOME" ]]; then
  echo "EDGE_HOME does not exist: $EDGE_HOME" >&2
  exit 1
fi

echo "==> Killing local tenex-edge processes..."
pkill -9 -x tenex-edge 2>/dev/null || true
sleep 0.5

echo "==> Wiping local state..."
rm -f "$EDGE_HOME/state.db" "$EDGE_HOME/state.db-shm" "$EDGE_HOME/state.db-wal"
rm -f "$EDGE_HOME/daemon.sock" "$EDGE_HOME/daemon.lock" "$EDGE_HOME/daemon.log"
rm -rf "$EDGE_HOME/sessions"
echo "    kept:"
find "$EDGE_HOME" -mindepth 1 -maxdepth 1 -print | sed 's/^/      /'

if [[ "$LOCAL_ONLY" == true ]]; then
  echo "==> Done (local only). Run: tenex-edge daemon start"
  exit 0
fi

echo "==> Wiping remote NIP-29 relay (nip29.f7z.io) on $RELAY_HOST..."
ssh "$RELAY_HOST" bash <<'REMOTE'
set -euo pipefail
sudo systemctl stop nip29-f7z-io
sudo rm -rf /opt/nip29-f7z-io/data/main \
            /opt/nip29-f7z-io/data/mmmm \
            /opt/nip29-f7z-io/data/global-search \
            /opt/nip29-f7z-io/data/events \
            /opt/nip29-f7z-io/data/mmmm.lock
sudo systemctl start nip29-f7z-io
sleep 1
systemctl is-active nip29-f7z-io
echo "    nip29-f7z-io restarted"
REMOTE

echo "==> Done. Run: tenex-edge daemon start"
