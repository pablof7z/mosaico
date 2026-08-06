#!/usr/bin/env bash
# Wipe Mosaico runtime state without touching configs or external relays.
# Usage: ./scripts/reset.sh --yes-i-know-this-wipes-local-state

set -euo pipefail

if [[ "${MOSAICO+x}" == x ]]; then
  if [[ "${MOSAICO_HOME+x}" == x ]]; then
    echo "MOSAICO cannot be combined with MOSAICO_HOME" >&2
    exit 2
  fi
  if [[ "${MOSAICO_CONFIG+x}" == x ]]; then
    echo "MOSAICO cannot be combined with MOSAICO_CONFIG" >&2
    exit 2
  fi
  if [[ ! "${MOSAICO}" =~ ^[a-z0-9][a-z0-9_-]{0,62}$ ]]; then
    echo "invalid MOSAICO instance name" >&2
    exit 2
  fi
  if [[ -z "${HOME:-}" ]]; then
    echo "HOME must be set when MOSAICO selects an instance" >&2
    exit 2
  fi
  if [[ "${HOME}" != /* ]]; then
    echo "HOME must be an absolute path when MOSAICO selects an instance" >&2
    exit 2
  fi
  if [[ "${MOSAICO}" == default ]]; then
    MOSAICO_HOME_DIR="${HOME}/.mosaico"
  else
    MOSAICO_HOME_DIR="${HOME}/.mosaico-instances/${MOSAICO}"
  fi
else
  if [[ "${MOSAICO_HOME+x}" == x ]]; then
    MOSAICO_HOME_DIR="${MOSAICO_HOME}"
    if [[ -z "${MOSAICO_HOME_DIR}" ]]; then
      echo "MOSAICO_HOME cannot be empty" >&2
      exit 2
    fi
  elif [[ -n "${HOME:-}" ]]; then
    MOSAICO_HOME_DIR="${HOME}/.mosaico"
  else
    echo "neither MOSAICO_HOME nor HOME is set" >&2
    exit 2
  fi
fi

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

# reset.sh is a WIPE tool: ask only the selected daemon to reap the PTY
# supervisors recorded in its selected home. Never kill by binary name or argv:
# another named instance may be running from the same executable.
if ! command -v mosaico >/dev/null 2>&1; then
  echo "mosaico is not on PATH; refusing an uncoordinated state wipe" >&2
  exit 1
fi
echo "==> Stopping selected Mosaico daemon and PTY supervisors..."
MOSAICO_REAP_SESSIONS_ON_STOP=1 mosaico daemon stop

echo "==> Wiping local state..."
rm -f "$MOSAICO_HOME_DIR/state.db" "$MOSAICO_HOME_DIR/state.db-shm" "$MOSAICO_HOME_DIR/state.db-wal"
rm -f "$MOSAICO_HOME_DIR/nmp.redb"
rm -f "$MOSAICO_HOME_DIR/daemon.sock" "$MOSAICO_HOME_DIR/daemon.lock" "$MOSAICO_HOME_DIR/daemon.log"
rm -f "$MOSAICO_HOME_DIR/daemon.inhibit"
rm -rf "$MOSAICO_HOME_DIR/sessions"
echo "    kept:"
find "$MOSAICO_HOME_DIR" -mindepth 1 -maxdepth 1 -print | sed 's/^/      /'

echo "==> Done. Run with the same instance selection: mosaico daemon restart"
