#!/usr/bin/env bash
set -euo pipefail

ROOT="$1"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

RESET_HOME="${TMP}/home"
RESET_BIN="${TMP}/bin"
RESET_LOG="${TMP}/command.log"
mkdir -p \
  "${RESET_HOME}/.mosaico" \
  "${RESET_HOME}/.mosaico-instances/relay1" \
  "${RESET_HOME}/.mosaico-instances/relay2" \
  "${RESET_BIN}"
touch \
  "${RESET_HOME}/.mosaico/state.db" \
  "${RESET_HOME}/.mosaico-instances/relay1/state.db" \
  "${RESET_HOME}/.mosaico-instances/relay2/state.db"
cat >"${RESET_BIN}/mosaico" <<'EOF'
#!/bin/sh
printf '%s|%s|%s\n' "${MOSAICO:-}" "${MOSAICO_REAP_SESSIONS_ON_STOP:-}" "$*" \
  >"${RESET_LOG}"
EOF
chmod +x "${RESET_BIN}/mosaico"

PATH="${RESET_BIN}:/usr/bin:/bin" HOME="${RESET_HOME}" MOSAICO=relay1 \
  RESET_LOG="${RESET_LOG}" \
  bash "${ROOT}/scripts/reset.sh" --yes-i-know-this-wipes-local-state >/dev/null
[[ "$(cat "${RESET_LOG}")" == \
  'relay1||daemon reset-state --yes-i-know-this-wipes-local-state' ]] \
  || fail 'script did not delegate the selected instance to the owned reset door'
[[ -e "${RESET_HOME}/.mosaico-instances/relay1/state.db" ]] \
  || fail 'script bypassed the mocked product reset and deleted state itself'
[[ -e "${RESET_HOME}/.mosaico/state.db" ]] \
  || fail 'named reset touched default instance state'
[[ -e "${RESET_HOME}/.mosaico-instances/relay2/state.db" ]] \
  || fail 'named reset touched another named instance state'
echo 'ok: reset script delegates exact selected-instance ownership to Mosaico'
