#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
SKILL="${ROOT}/skills/tenex-edge-dev"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_eq() {
  local expected="$1" actual="$2" label="$3"
  [[ "${actual}" == "${expected}" ]] \
    || fail "${label}: expected [${expected}], got [${actual}]"
  echo "ok: ${label}"
}

write_profile() {
  local state="$1" agent="$2" bundle="$3" transport="$4"
  mkdir -p "${state}/tenex/edge/agents"
  printf '{"slug":"%s","harness":"%s"}\n' "${agent}" "${bundle}" \
    >"${state}/tenex/edge/agents/${agent}.json"
  printf '{"%s":{"harness":"%s","transport":"%s"}}\n' \
    "${bundle}" "${agent}" "${transport}" >"${state}/tenex/edge/harnesses.json"
}

write_lab_env() {
  local path="$1" state="$2"
  {
    printf 'RUN_ID=%q\n' test-run
    printf 'WORK_DIR=%q\n' "${TMP}/work"
    printf 'TENEX_EDGE_CONTAINER_STATE=%q\n' "${state}"
  } >"${path}"
}

launch_tail() {
  awk 'seen || $0 == "<launch>" { seen = 1; print }'
}

mkdir -p "${TMP}/launcher-bin" "${TMP}/work"
cat >"${TMP}/launcher-bin/env" <<'EOF'
#!/bin/sh
for arg in "$@"; do
  printf '<%s>\n' "$arg"
done
EOF
chmod +x "${TMP}/launcher-bin/env"

ACP_STATE="${TMP}/claude-acp-state"
ACP_ENV="${TMP}/claude-acp.env"
write_profile "${ACP_STATE}" claude claude-acp acp
write_lab_env "${ACP_ENV}" "${ACP_STATE}"
ACP_OUTPUT="$(
  PATH="${TMP}/launcher-bin:${PATH}" \
    TENEX_EDGE_DEV_PROMPT='prompt with spaces' \
    bash "${SKILL}/scripts/launch-agent" "${ACP_ENV}" launch claude-acp
)"
ACP_TAIL="$(printf '%s\n' "${ACP_OUTPUT}" | launch_tail | sed -n '1,3p')"
assert_eq $'<launch>\n<claude>\n<prompt with spaces>' "${ACP_TAIL}" \
  'ACP launch uses the positional prompt contract'
if printf '%s\n' "${ACP_OUTPUT}" | grep -Fq -- '--prompt'; then
  fail 'ACP launch still emits removed --prompt flag'
fi

PTY_STATE="${TMP}/claude-state"
PTY_ENV="${TMP}/claude.env"
write_profile "${PTY_STATE}" claude claude pty
write_lab_env "${PTY_ENV}" "${PTY_STATE}"
PTY_OUTPUT="$(
  PATH="${TMP}/launcher-bin:${PATH}" \
    TENEX_EDGE_DEV_PROMPT='inspect identity' \
    bash "${SKILL}/scripts/launch-agent" "${PTY_ENV}" launch claude --model haiku
)"
PTY_TAIL="$(printf '%s\n' "${PTY_OUTPUT}" | launch_tail | sed -n '1,6p')"
assert_eq $'<launch>\n<claude>\n<inspect identity>\n<-->\n<--model>\n<haiku>' \
  "${PTY_TAIL}" 'PTY prompt precedes provider arguments'

mkdir -p "${TMP}/relay-bin" "${TMP}/croissant"
cat >"${TMP}/relay-bin/curl" <<'EOF'
#!/bin/sh
exit 1
EOF
cat >"${TMP}/relay-bin/lsof" <<'EOF'
#!/bin/sh
exit 1
EOF
cat >"${TMP}/relay-bin/nak" <<'EOF'
#!/bin/sh
if [ "${1:-} ${2:-}" = 'key generate' ]; then
  echo nsec-test
elif [ "${1:-} ${2:-}" = 'key public' ]; then
  printf '%064d\n' 0
else
  exit 2
fi
EOF
cat >"${TMP}/croissant/croissant" <<'EOF'
#!/bin/sh
exec /bin/sleep 60
EOF
chmod +x "${TMP}/relay-bin/curl" "${TMP}/relay-bin/lsof" \
  "${TMP}/relay-bin/nak" "${TMP}/croissant/croissant"

set +e
RELAY_OUTPUT="$(
  PATH="${TMP}/relay-bin:${PATH}" \
    TENEX_EDGE_DEV_CROISSANT_DIR="${TMP}/croissant" \
    TENEX_EDGE_DEV_RELAY_HOST=127.0.0.1 \
    TENEX_EDGE_DEV_RELAY_PORT=29999 \
    TENEX_EDGE_DEV_RELAY_READY_TIMEOUT=1 \
    TENEX_EDGE_DEV_WORK="${TMP}/relay-work" \
    bash "${SKILL}/scripts/start-croissant-relay" 2>&1
)"
RELAY_STATUS=$?
set -e
[[ "${RELAY_STATUS}" -ne 0 ]] || fail 'unreachable relay unexpectedly passed readiness'
grep -Fq 'relay did not become ready' <<<"${RELAY_OUTPUT}" \
  || fail 'readiness failure did not report the relay URL'
RELAY_PID="$(cat "${TMP}/relay-work/relay.pid")"
if kill -0 "${RELAY_PID}" 2>/dev/null; then
  kill "${RELAY_PID}" 2>/dev/null || true
  fail "readiness failure leaked relay pid ${RELAY_PID}"
fi
echo 'ok: readiness failure reaps the relay process'

bash -n "${SKILL}"/scripts/* "${ROOT}/containers/tenex-edge/doctor"
echo 'ok: skill and doctor scripts parse as bash'
