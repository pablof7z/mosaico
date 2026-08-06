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
  "${RESET_HOME}/.mosaico-instances/relay1/sessions" \
  "${RESET_HOME}/.mosaico-instances/relay2" \
  "${RESET_BIN}"
touch \
  "${RESET_HOME}/.mosaico/state.db" \
  "${RESET_HOME}/.mosaico-instances/relay1/state.db" \
  "${RESET_HOME}/.mosaico-instances/relay1/sessions/owned" \
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
[[ "$(cat "${RESET_LOG}")" == 'relay1|1|daemon stop' ]] \
  || fail 'reset did not stop only the selected daemon with supervisor reaping'
[[ ! -e "${RESET_HOME}/.mosaico-instances/relay1/state.db" ]] \
  || fail 'reset left selected named state behind'
[[ ! -e "${RESET_HOME}/.mosaico-instances/relay1/sessions" ]] \
  || fail 'reset left selected named sessions behind'
[[ -e "${RESET_HOME}/.mosaico/state.db" ]] \
  || fail 'named reset touched default instance state'
[[ -e "${RESET_HOME}/.mosaico-instances/relay2/state.db" ]] \
  || fail 'named reset touched another named instance state'
echo 'ok: reset stops only the selected daemon and preserves every other instance'
