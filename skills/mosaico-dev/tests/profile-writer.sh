# shellcheck shell=bash

run_profile_writer_tests() {
  mkdir -p "${TMP}/writer-bin" "${TMP}/writer-work/keys"
  printf 'nsec-relay-owner\n' >"${TMP}/writer-work/keys/relay-owner.nsec"
  write_fake_nak
  local writer_env="${TMP}/writer.env"
  cat >"${TMP}/writer-work/humans.json" <<EOF
[
  {"number":1,"name":"Pablo","pubkey":"pub-relay-owner","secret_file":"${TMP}/writer-work/keys/relay-owner.nsec"},
  {"number":2,"name":"Alice","pubkey":"pub-human-2","secret_file":"${TMP}/writer-work/keys/human-2.nsec"},
  {"number":3,"name":"Bob","pubkey":"pub-human-3","secret_file":"${TMP}/writer-work/keys/human-3.nsec"}
]
EOF
  {
    printf 'RUN_ID=%q\n' test-run
    printf 'WORK_DIR=%q\n' "${TMP}/writer-work"
    printf 'RELAY_WS=%q\n' 'ws://127.0.0.1:19888'
    printf 'OWNER_SK_FILE=%q\n' "${TMP}/writer-work/keys/relay-owner.nsec"
    printf 'HUMAN_IDENTITIES_FILE=%q\n' "${TMP}/writer-work/humans.json"
  } >"${writer_env}"

  local writer_output
  writer_output="$(
    PATH="${TMP}/writer-bin:${PATH}" \
      NAK_COUNTER_FILE="${TMP}/nak-counter" \
      MOSAICO_DEV_STATE_ROOT="${TMP}/container-state" \
      MOSAICO_DEV_CODEX_CONFIG_PROFILE=planner \
      MOSAICO_DEV_HERMES_PROFILE=reviewer \
      MOSAICO_DEV_CODEX_APP_SERVER_ARGS_JSON='["--strict-config"]' \
      bash "${SKILL}/scripts/write-container-profiles" "${writer_env}" \
        claude-acp codex-app-server grok goose-acp hermes hermes-acp \
        codex-ollama opencode-ollama
  )"
  assert_generated_profiles
  assert_regeneration_preserves_key "${writer_env}"
  if grep -Fq 'nsec-' <<<"${writer_output}"; then
    fail 'profile writer leaked secret key material'
  fi
  echo 'ok: profile writer output does not expose secrets'
  assert_bad_args_rejected "${writer_env}"
}

write_fake_nak() {
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
}

assert_generated_profiles() {
  local profile harnesses agent config
  for profile in claude-acp codex-app-server grok goose-acp hermes hermes-acp \
    codex-ollama opencode-ollama; do
    harnesses="${TMP}/container-state/${profile}/mosaico/harnesses.json"
    agent="$(find "${TMP}/container-state/${profile}/mosaico/agents" \
      -type f -name '*.json')"
    config="${TMP}/container-state/${profile}/mosaico/config.json"
    assert_json 'all(.[]; ((keys - ["args","harness","transport"]) | length) == 0)' \
      "${harnesses}" "${profile} bundle contains only current fields"
    assert_json 'has("slug") and has("created_at") and .perSessionKey == true and has("harness") and (has("secret_key") | not) and (has("public_key") | not)' \
      "${agent}" "${profile} agent is keyless"
    assert_json '.userNsec == "nsec-relay-owner" and .whitelistedPubkeys == ["pub-relay-owner","pub-human-2","pub-human-3"] and (.mosaicoPrivateKey != .userNsec)' \
      "${config}" "${profile} separates human and backend keys"
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
  assert_json '.["goose-acp"] == {"harness":"goose","transport":"acp"}' \
    "${TMP}/container-state/goose-acp/mosaico/harnesses.json" \
    'Goose profile emits a native ACP bundle'
  assert_json '.["hermes"] == {"harness":"hermes","transport":"pty"}' \
    "${TMP}/container-state/hermes/mosaico/harnesses.json" \
    'Hermes profile emits a native PTY bundle'
  assert_json '.["hermes-acp"] == {"harness":"hermes","transport":"acp"}' \
    "${TMP}/container-state/hermes-acp/mosaico/harnesses.json" \
    'Hermes ACP profile emits a structured bundle'
  assert_json '.profile == "reviewer"' \
    "${TMP}/container-state/hermes-acp/mosaico/agents/hermes.json" \
    'Hermes named profile belongs to agent config'
  assert_json '.profile == "planner"' \
    "${TMP}/container-state/codex-app-server/mosaico/agents/codex.json" \
    'Codex named profile belongs to agent config'
  assert_json '.["codex-ollama"].args == ["--oss","--local-provider","ollama"]' \
    "${TMP}/container-state/codex-ollama/mosaico/harnesses.json" \
    'Codex Ollama bundle owns provider args'
  assert_json '.["opencode-ollama"].args == ["-m","ollama/deepseek-r1:8b"]' \
    "${TMP}/container-state/opencode-ollama/mosaico/harnesses.json" \
    'OpenCode Ollama bundle owns model args'
  local key_count
  key_count="$(
    for profile in claude-acp codex-app-server grok goose-acp hermes hermes-acp \
      codex-ollama opencode-ollama; do
      jq -r '.mosaicoPrivateKey' \
        "${TMP}/container-state/${profile}/mosaico/config.json"
    done | sort -u | wc -l | tr -d ' '
  )"
  assert_eq 8 "${key_count}" 'each profile has a distinct backend key'
}

assert_regeneration_preserves_key() {
  local writer_env="$1" before
  before="$(<"${TMP}/writer-work/keys/claude-acp.nsec")"
  PATH="${TMP}/writer-bin:${PATH}" \
    NAK_COUNTER_FILE="${TMP}/nak-counter" \
    MOSAICO_DEV_STATE_ROOT="${TMP}/container-state" \
    bash "${SKILL}/scripts/write-container-profiles" "${writer_env}" claude-acp \
    >/dev/null
  assert_eq "${before}" "$(<"${TMP}/writer-work/keys/claude-acp.nsec")" \
    'profile regeneration preserves backend key material'
}

assert_bad_args_rejected() {
  local writer_env="$1" output status
  set +e
  output="$(
    PATH="${TMP}/writer-bin:${PATH}" \
      NAK_COUNTER_FILE="${TMP}/nak-counter" \
      MOSAICO_DEV_STATE_ROOT="${TMP}/bad-state" \
      MOSAICO_DEV_CLAUDE_ACP_ARGS_JSON='{"model":"haiku"}' \
      bash "${SKILL}/scripts/write-container-profiles" "${writer_env}" claude-acp 2>&1
  )"
  status=$?
  set -e
  [[ "${status}" -eq 2 ]] || fail 'non-array bundle args unexpectedly passed'
  grep -Fq 'expected an array of strings' <<<"${output}" \
    || fail 'invalid args JSON did not report the current contract'
  echo 'ok: profile writer rejects obsolete object-shaped bundle config'
}
