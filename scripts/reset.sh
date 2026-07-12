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

# reset.sh is a WIPE tool: it deletes state.db AND the sessions dir, so any
# surviving PTY supervisor would be orphaned against a wiped DB — hence a reset
# reaps the supervisors too. Do NOT copy this kill into a plain daemon *restart*:
# the daemon and every detached PTY supervisor are the SAME binary (`tenex-edge`),
# so a bare `pkill -x tenex-edge` reaps live agent sessions along with the daemon.
# A restart must kill ONLY the daemon (`pkill -f 'tenex-edge daemon'`); the daemon
# then re-adopts the still-running supervisors on boot (reconcile_sessions), and a
# systemd unit must use `KillMode=process`. Target argv explicitly here instead of
# the shared binary name so we never reap an unrelated `tenex-edge` on the box.
echo "==> Killing local tenex-edge daemon..."
pkill -9 -f 'tenex-edge daemon' 2>/dev/null || true
echo "==> Killing local tenex-edge PTY supervisors (state is being wiped)..."
pkill -9 -f 'tenex-edge __pty-supervisor' 2>/dev/null || true
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
