#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
SKILL="${ROOT}/skills/mosaico-dev"
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
  mkdir -p "${state}/mosaico/agents"
  printf '{"slug":"%s","harness":"%s"}\n' "${agent}" "${bundle}" \
    >"${state}/mosaico/agents/${agent}.json"
  printf '{"%s":{"harness":"%s","transport":"%s"}}\n' \
    "${bundle}" "${agent}" "${transport}" >"${state}/mosaico/harnesses.json"
}

write_lab_env() {
  local path="$1" state="$2"
  {
    printf 'RUN_ID=%q\n' test-run
    printf 'WORK_DIR=%q\n' "${TMP}/work"
    printf 'RELAY_WS=%q\n' ws://127.0.0.1:29999
    printf 'MOSAICO_CONTAINER_STATE=%q\n' "${state}"
  } >"${path}"
}

assert_json() {
  local filter="$1" path="$2" label="$3"
  jq -e "${filter}" "${path}" >/dev/null || fail "${label}: ${path}"
  echo "ok: ${label}"
}

agents_tail() {
  awk 'seen || $0 == "<agents>" { seen = 1; print }'
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
    MOSAICO_DEV_PROMPT='prompt with spaces' \
    bash "${SKILL}/scripts/launch-agent" "${ACP_ENV}" launch claude-acp
)"
ACP_TAIL="$(printf '%s\n' "${ACP_OUTPUT}" | agents_tail | sed -n '1,3p')"
assert_eq $'<agents>\n<claude>\n<prompt with spaces>' "${ACP_TAIL}" \
  'ACP launch uses the positional prompt contract'
if printf '%s\n' "${ACP_OUTPUT}" | grep -Eq '^<--(prompt|headless)>$'; then
  fail 'ACP launch emits a removed launch flag'
fi
echo 'ok: ACP launch emits no removed flags'

PTY_STATE="${TMP}/claude-state"
PTY_ENV="${TMP}/claude.env"
write_profile "${PTY_STATE}" claude claude pty
write_lab_env "${PTY_ENV}" "${PTY_STATE}"
PTY_OUTPUT="$(
  PATH="${TMP}/launcher-bin:${PATH}" \
    MOSAICO_DEV_PROMPT='inspect identity' \
    bash "${SKILL}/scripts/launch-agent" "${PTY_ENV}" launch claude
)"
PTY_TAIL="$(printf '%s\n' "${PTY_OUTPUT}" | agents_tail | sed -n '1,3p')"
assert_eq $'<agents>\n<claude>\n<inspect identity>' \
  "${PTY_TAIL}" 'PTY launch uses only target and positional prompt'

set +e
PTY_ARGS_OUTPUT="$(
  PATH="${TMP}/launcher-bin:${PATH}" \
    bash "${SKILL}/scripts/launch-agent" "${PTY_ENV}" launch claude --model haiku 2>&1
)"
PTY_ARGS_STATUS=$?
set -e
[[ "${PTY_ARGS_STATUS}" -eq 2 ]] || fail 'launch provider arguments were not rejected'
grep -Fq 'put operational args in the harness bundle' <<<"${PTY_ARGS_OUTPUT}" \
  || fail 'launch argument rejection did not explain the bundle contract'
echo 'ok: launch rejects provider arguments'

DIRECT_OUTPUT="$(
  PATH="${TMP}/launcher-bin:${PATH}" \
    bash "${SKILL}/scripts/launch-agent" "${PTY_ENV}" direct claude --model haiku
)"
DIRECT_TAIL="$(printf '%s\n' "${DIRECT_OUTPUT}" \
  | awk '$0 == "<claude>" { count++ } count == 2 { print }' | sed -n '1,3p')"
assert_eq $'<claude>\n<--model>\n<haiku>' "${DIRECT_TAIL}" \
  'direct mode still forwards provider arguments'

mkdir -p "${TMP}/writer-bin" "${TMP}/writer-work/keys"
printf 'nsec-relay-owner\n' >"${TMP}/writer-work/keys/relay-owner.nsec"
cat >"${TMP}/writer-bin/nak" <<'EOF'
#!/bin/sh
if [ "${1:-} ${2:-}" = 'key generate' ]; then
  count=0
  [ ! -f "${NAK_COUNTER_FILE}" ] || count="$(cat "${NAK_COUNTER_FILE}")"
  count=$((count + 1))
  printf '%s\n' "${count}" >"${NAK_COUNTER_FILE}"
  printf 'nsec-backend-%s\n' "${count}"
elif [ "${1:-} ${2:-}" = 'key public' ]; then
  case "${3:-}" in
    nsec-relay-owner) printf 'pub-relay-owner\n' ;;
    nsec-backend-*) printf 'pub-backend-%s\n' "${3##*-}" ;;
    *) exit 2 ;;
  esac
else
  exit 2
fi
EOF
chmod +x "${TMP}/writer-bin/nak"
WRITER_ENV="${TMP}/writer.env"
{
  printf 'RUN_ID=%q\n' test-run
  printf 'WORK_DIR=%q\n' "${TMP}/writer-work"
  printf 'RELAY_WS=%q\n' 'ws://127.0.0.1:19888'
  printf 'OWNER_SK_FILE=%q\n' "${TMP}/writer-work/keys/relay-owner.nsec"
} >"${WRITER_ENV}"
WRITER_OUTPUT="$(
  PATH="${TMP}/writer-bin:${PATH}" \
    NAK_COUNTER_FILE="${TMP}/nak-counter" \
    MOSAICO_DEV_STATE_ROOT="${TMP}/container-state" \
    MOSAICO_DEV_CODEX_CONFIG_PROFILE=planner \
    MOSAICO_DEV_CODEX_APP_SERVER_ARGS_JSON='["--strict-config"]' \
    bash "${SKILL}/scripts/write-container-profiles" "${WRITER_ENV}" \
      claude-acp codex-app-server grok codex-ollama opencode-ollama
)"

for profile in claude-acp codex-app-server grok codex-ollama opencode-ollama; do
  HARNESSES="${TMP}/container-state/${profile}/mosaico/harnesses.json"
  AGENT="$(find "${TMP}/container-state/${profile}/mosaico/agents" -type f -name '*.json')"
  CONFIG="${TMP}/container-state/${profile}/mosaico/config.json"
  assert_json 'all(.[]; ((keys - ["args","harness","transport"]) | length) == 0)' \
    "${HARNESSES}" "${profile} bundle contains only current fields"
  assert_json 'has("slug") and has("created_at") and .perSessionKey == true and has("harness") and (has("secret_key") | not) and (has("public_key") | not)' \
    "${AGENT}" "${profile} agent is keyless"
  assert_json '.userNsec == "nsec-relay-owner" and .whitelistedPubkeys == ["pub-relay-owner"] and (.mosaicoPrivateKey != .userNsec)' \
    "${CONFIG}" "${profile} separates human and backend keys"
done

assert_json '.["claude-acp"] == {"harness":"claude-code","transport":"acp"}' \
  "${TMP}/container-state/claude-acp/mosaico/harnesses.json" \
  'structured bundle defaults to no args'
assert_json '.["codex-app-server"].args == ["--strict-config"]' \
  "${TMP}/container-state/codex-app-server/mosaico/harnesses.json" \
  'per-profile args JSON overrides defaults'
assert_json '.["grok"] == {"harness":"grok","transport":"pty"}' \
  "${TMP}/container-state/grok/mosaico/harnesses.json" \
  'Grok profile emits a native PTY bundle'
assert_json '.profile == "planner"' \
  "${TMP}/container-state/codex-app-server/mosaico/agents/codex.json" \
  'Codex named profile belongs to agent config'
assert_json '.["codex-ollama"].args == ["--oss","--local-provider","ollama"]' \
  "${TMP}/container-state/codex-ollama/mosaico/harnesses.json" \
  'Codex Ollama bundle owns provider args'
assert_json '.["opencode-ollama"].args == ["-m","ollama/deepseek-r1:8b"]' \
  "${TMP}/container-state/opencode-ollama/mosaico/harnesses.json" \
  'OpenCode Ollama bundle owns model args'

BACKEND_KEY_COUNT="$(
  for profile in claude-acp codex-app-server grok codex-ollama opencode-ollama; do
    jq -r '.mosaicoPrivateKey' \
      "${TMP}/container-state/${profile}/mosaico/config.json"
  done | sort -u | wc -l | tr -d ' '
)"
assert_eq 5 "${BACKEND_KEY_COUNT}" 'each profile has a distinct backend key'
CLAUDE_BACKEND_BEFORE="$(<"${TMP}/writer-work/keys/claude-acp.nsec")"
PATH="${TMP}/writer-bin:${PATH}" \
  NAK_COUNTER_FILE="${TMP}/nak-counter" \
  MOSAICO_DEV_STATE_ROOT="${TMP}/container-state" \
  bash "${SKILL}/scripts/write-container-profiles" "${WRITER_ENV}" claude-acp \
  >/dev/null
assert_eq "${CLAUDE_BACKEND_BEFORE}" \
  "$(<"${TMP}/writer-work/keys/claude-acp.nsec")" \
  'profile regeneration preserves backend key material'

if grep -Fq 'nsec-' <<<"${WRITER_OUTPUT}"; then
  fail 'profile writer leaked secret key material'
fi
echo 'ok: profile writer output does not expose secrets'

set +e
BAD_ARGS_OUTPUT="$(
  PATH="${TMP}/writer-bin:${PATH}" \
    NAK_COUNTER_FILE="${TMP}/nak-counter" \
    MOSAICO_DEV_STATE_ROOT="${TMP}/bad-state" \
    MOSAICO_DEV_CLAUDE_ACP_ARGS_JSON='{"model":"haiku"}' \
    bash "${SKILL}/scripts/write-container-profiles" "${WRITER_ENV}" claude-acp 2>&1
)"
BAD_ARGS_STATUS=$?
set -e
[[ "${BAD_ARGS_STATUS}" -eq 2 ]] || fail 'non-array bundle args unexpectedly passed'
grep -Fq 'expected an array of strings' <<<"${BAD_ARGS_OUTPUT}" \
  || fail 'invalid args JSON did not report the current contract'
echo 'ok: profile writer rejects obsolete object-shaped bundle config'

GROK_STATE="${TMP}/grok-state"
GROK_ENV="${TMP}/grok.env"
write_profile "${GROK_STATE}" grok grok pty
write_lab_env "${GROK_ENV}" "${GROK_STATE}"
GROK_OUTPUT="$(
  PATH="${TMP}/launcher-bin:${PATH}" \
    MOSAICO_DEV_PROMPT='inspect grok identity' \
    bash "${SKILL}/scripts/launch-agent" "${GROK_ENV}" launch grok
)"
GROK_TAIL="$(printf '%s\n' "${GROK_OUTPUT}" | agents_tail | sed -n '1,3p')"
assert_eq $'<agents>\n<grok>\n<inspect grok identity>' \
  "${GROK_TAIL}" 'Grok uses the current PTY launch contract'

HOST_HOME="${TMP}/host-home"
STATE_DIR="${TMP}/host-auth-state"
mkdir -p "${HOST_HOME}/.codex" "${STATE_DIR}/home/.codex"
printf 'model = "profile-model"\n' >"${HOST_HOME}/.codex/planner.config.toml"
export HOST_AUTH=1
# shellcheck source=/dev/null
source "${ROOT}/containers/mosaico/host-auth.bash"
stage_codex_named_profiles
assert_eq '/host-auth/codex/planner.config.toml' \
  "$(readlink "${STATE_DIR}/home/.codex/planner.config.toml")" \
  'host auth stages named Codex profiles'
rm -f "${HOST_HOME}/.codex/planner.config.toml"
stage_codex_named_profiles
if [[ -e "${STATE_DIR}/home/.codex/planner.config.toml" \
  || -L "${STATE_DIR}/home/.codex/planner.config.toml" ]]; then
  fail 'removed host Codex profile left a stale staged symlink'
fi
echo 'ok: host auth removes stale named Codex profile symlinks'

mkdir -p "${HOST_HOME}/.grok" "${STATE_DIR}/home/.grok"
printf '{"token":"secret-test-value"}\n' >"${HOST_HOME}/.grok/auth.json"
printf 'theme = "dark"\n' >"${HOST_HOME}/.grok/config.toml"
export AGENT=grok
stage_grok_state
build_host_auth_mounts
cmp -s "${HOST_HOME}/.grok/auth.json" "${STATE_DIR}/home/.grok/auth.json" \
  || fail 'Grok auth was not copied into isolated state'
cmp -s "${HOST_HOME}/.grok/config.toml" "${STATE_DIR}/home/.grok/config.toml" \
  || fail 'Grok config was not copied into isolated state'
[[ ! -L "${STATE_DIR}/home/.grok/auth.json" ]] \
  || fail 'Grok auth must be writable isolated state, not a host symlink'
[[ "${#HOST_AUTH_MOUNTS[@]}" -eq 0 ]] \
  || fail 'Grok host auth unexpectedly exposed a host bind mount'
echo 'ok: host auth copies Grok state without sharing the host file'

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
    MOSAICO_DEV_CROISSANT_DIR="${TMP}/croissant" \
    MOSAICO_DEV_RELAY_HOST=127.0.0.1 \
    MOSAICO_DEV_RELAY_PORT=29999 \
    MOSAICO_DEV_RELAY_READY_TIMEOUT=1 \
    MOSAICO_DEV_WORK="${TMP}/relay-work" \
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

cat >"${TMP}/relay-bin/curl" <<'EOF'
#!/bin/sh
exit 0
EOF
FOREGROUND_WORK="${TMP}/foreground-relay"
PATH="${TMP}/relay-bin:${PATH}" \
  MOSAICO_DEV_CROISSANT_DIR="${TMP}/croissant" \
  MOSAICO_DEV_RELAY_HOST=127.0.0.1 \
  MOSAICO_DEV_RELAY_PORT=29998 \
  MOSAICO_DEV_RELAY_FOREGROUND=1 \
  MOSAICO_DEV_WORK="${FOREGROUND_WORK}" \
  bash "${SKILL}/scripts/start-croissant-relay" \
    >"${TMP}/foreground-relay.out" 2>&1 &
FOREGROUND_HELPER_PID=$!
for _ in 1 2 3 4 5; do
  [[ -s "${FOREGROUND_WORK}/relay.pid" ]] && break
  sleep 1
done
[[ -s "${FOREGROUND_WORK}/relay.pid" ]] \
  || fail 'foreground relay did not write its pid file'
FOREGROUND_RELAY_PID="$(cat "${FOREGROUND_WORK}/relay.pid")"
kill -0 "${FOREGROUND_HELPER_PID}" 2>/dev/null \
  || fail 'foreground relay helper returned instead of remaining yielded'
kill "${FOREGROUND_RELAY_PID}"
wait "${FOREGROUND_HELPER_PID}"
grep -Fq 'relay_foreground=1' "${TMP}/foreground-relay.out" \
  || fail 'foreground relay did not report its persistent mode'
echo 'ok: foreground relay mode remains yielded until cleanup stops the relay'

bash -n "${SKILL}"/scripts/* "${ROOT}/containers/mosaico/doctor" \
  "${ROOT}/containers/mosaico/host-auth.bash"
echo 'ok: skill and container helper scripts parse as bash'

cargo test --quiet --lib harness::tests::config_accepts_only_harness_transport_and_args
cargo test --quiet --lib identity::tests::creates_then_reloads_keyless_agent_config
cargo test --quiet --lib config::tests::key_accessors_split_when_both_present
echo 'ok: generated config assumptions match current Rust schemas'
