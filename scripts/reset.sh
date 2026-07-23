#!/usr/bin/env bash
# Wipe Mosaico runtime state without touching configs or external relays.
# Usage: ./scripts/reset.sh --yes-i-know-this-wipes-local-state

set -euo pipefail

MOSAICO_HOME_DIR="${MOSAICO_HOME:-$HOME/.mosaico}"

case "${1:-}" in
  --yes-i-know-this-wipes-local-state)
    ;;
  *)
    cat >&2 <<EOF
Refusing to reset without explicit confirmation.

This deletes local runtime state under:
  $MOSAICO_HOME_DIR

Options:
  $0 --yes-i-know-this-wipes-local-state
      Wipe local state (db, sessions, sockets). External relays are untouched.
EOF
    exit 2
    ;;
esac

if [[ ! -d "$MOSAICO_HOME_DIR" ]]; then
  echo "MOSAICO_HOME_DIR does not exist: $MOSAICO_HOME_DIR" >&2
  exit 1
fi

# reset.sh is a WIPE tool: it deletes state.db AND the sessions dir, so any
# surviving PTY supervisor would be orphaned against a wiped DB — hence a reset
# reaps the supervisors too. Do NOT copy this kill into a plain daemon *restart*:
# the daemon and every detached PTY supervisor are the SAME binary (`mosaico`),
# so a bare `pkill -x mosaico` reaps live agent sessions along with the daemon.
# A restart must kill ONLY the daemon (`pkill -f 'mosaico daemon'`); the daemon
# then re-adopts the still-running supervisors on boot (reconcile_sessions), and a
# systemd unit must use `KillMode=process`. Target argv explicitly here instead of
# the shared binary name so we never reap an unrelated `mosaico` on the box.
echo "==> Killing local mosaico daemon..."
pkill -9 -f 'mosaico daemon' 2>/dev/null || true
echo "==> Killing local mosaico PTY supervisors (state is being wiped)..."
pkill -9 -f 'mosaico __pty-supervisor' 2>/dev/null || true
sleep 0.5

echo "==> Wiping local state..."
rm -f "$MOSAICO_HOME_DIR/state.db" "$MOSAICO_HOME_DIR/state.db-shm" "$MOSAICO_HOME_DIR/state.db-wal"
rm -f "$MOSAICO_HOME_DIR/nmp.redb"
rm -f "$MOSAICO_HOME_DIR/daemon.sock" "$MOSAICO_HOME_DIR/daemon.lock" "$MOSAICO_HOME_DIR/daemon.log"
rm -rf "$MOSAICO_HOME_DIR/sessions"
echo "    kept:"
find "$MOSAICO_HOME_DIR" -mindepth 1 -maxdepth 1 -print | sed 's/^/      /'

echo "==> Done. Run: mosaico daemon start"
