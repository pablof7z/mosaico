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
      MOSAICO_DEV_KIMI_PROFILE=reviewer \
      MOSAICO_DEV_CODEX_APP_SERVER_ARGS_JSON='["--strict-config"]' \
      bash "${SKILL}/scripts/write-container-profiles" "${writer_env}" \
        claude-acp codex-app-server grok goose goose-acp hermes hermes-acp \
        kimi kimi-acp pi pi-rpc codex-ollama opencode-ollama
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
  local profile presets agent config
  for profile in claude-acp codex-app-server grok goose goose-acp hermes hermes-acp \
    kimi kimi-acp pi pi-rpc codex-ollama opencode-ollama; do
    presets="${TMP}/container-state/${profile}/mosaico/presets.json"
    agent="$(find "${TMP}/container-state/${profile}/mosaico/agents" \
      -type f -name '*.json')"
    config="${TMP}/container-state/${profile}/mosaico/config.json"
    assert_json 'all(.[]; type == "object" and all(.[]; type == "object" and all(.[]; type == "array")))' \
      "${presets}" "${profile} preset contains transport argument arrays"
    assert_json 'has("slug") and has("created_at") and .perSessionKey == true and has("harness") and has("preset") and (has("secret_key") | not) and (has("public_key") | not)' \
      "${agent}" "${profile} agent is keyless"
    assert_json '.userNsec == "nsec-relay-owner" and .whitelistedPubkeys == ["pub-relay-owner","pub-human-2","pub-human-3"] and (.mosaicoPrivateKey != .userNsec)' \
      "${config}" "${profile} separates human and backend keys"
  done

  assert_json '.["claude-acp"]["claude-code"].acp == []' \
    "${TMP}/container-state/claude-acp/mosaico/presets.json" \
    'structured preset defaults to no args'
  assert_json '.["codex-app-server"].codex["app-server"] == ["--strict-config"]' \
    "${TMP}/container-state/codex-app-server/mosaico/presets.json" \
    'per-profile args JSON overrides defaults'
  assert_json '.["grok"].grok.pty == []' \
    "${TMP}/container-state/grok/mosaico/presets.json" \
    'Grok profile emits PTY preset args'
  assert_json '.["goose"].goose.pty == []' \
    "${TMP}/container-state/goose/mosaico/presets.json" \
    'Goose profile emits interactive PTY preset args'
  assert_json '.["goose-acp"].goose.acp == []' \
    "${TMP}/container-state/goose-acp/mosaico/presets.json" \
    'Goose profile emits ACP preset args'
  assert_json '.["hermes"].hermes.pty == []' \
    "${TMP}/container-state/hermes/mosaico/presets.json" \
    'Hermes profile emits PTY preset args'
  assert_json '.["hermes-acp"].hermes.acp == []' \
    "${TMP}/container-state/hermes-acp/mosaico/presets.json" \
    'Hermes ACP profile emits structured preset args'
  assert_json '.profile == "reviewer"' \
    "${TMP}/container-state/hermes-acp/mosaico/agents/hermes.json" \
    'Hermes named profile belongs to agent config'
  assert_json '.["kimi"].kimi.pty == []' \
    "${TMP}/container-state/kimi/mosaico/presets.json" \
    'Kimi profile emits interactive PTY preset args'
  assert_json '.profile == "reviewer"' \
    "${TMP}/container-state/kimi/mosaico/agents/kimi.json" \
    'Kimi PTY named profile belongs to agent config'
  assert_json '.["kimi-acp"].kimi.acp == []' \
    "${TMP}/container-state/kimi-acp/mosaico/presets.json" \
    'Kimi ACP profile emits structured preset args'
  assert_json 'has("profile") | not' \
    "${TMP}/container-state/kimi-acp/mosaico/agents/kimi.json" \
    'Kimi ACP omits unsupported named profiles'
  assert_json '.["pi"].pi.pty == []' \
    "${TMP}/container-state/pi/mosaico/presets.json" \
    'Pi profile emits interactive PTY preset args'
  assert_json '.["pi-rpc"].pi["pi-rpc"] == []' \
    "${TMP}/container-state/pi-rpc/mosaico/presets.json" \
    'Pi RPC profile emits managed preset args'
  assert_json '.profile == "planner"' \
    "${TMP}/container-state/codex-app-server/mosaico/agents/codex.json" \
    'Codex named profile belongs to agent config'
  assert_json '.["codex-ollama"].codex.pty == ["--oss","--local-provider","ollama"]' \
    "${TMP}/container-state/codex-ollama/mosaico/presets.json" \
    'Codex Ollama preset owns provider args'
  assert_json '.["opencode-ollama"].opencode.pty == ["-m","ollama/deepseek-r1:8b"]' \
    "${TMP}/container-state/opencode-ollama/mosaico/presets.json" \
    'OpenCode Ollama preset owns model args'
  local key_count
  key_count="$(
    for profile in claude-acp codex-app-server grok goose goose-acp hermes hermes-acp \
      kimi kimi-acp pi pi-rpc codex-ollama opencode-ollama; do
      jq -r '.mosaicoPrivateKey' \
        "${TMP}/container-state/${profile}/mosaico/config.json"
    done | sort -u | wc -l | tr -d ' '
  )"
  assert_eq 13 "${key_count}" 'each profile has a distinct backend key'
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
  [[ "${status}" -eq 2 ]] || fail 'non-array preset args unexpectedly passed'
  grep -Fq 'expected an array of strings' <<<"${output}" \
    || fail 'invalid args JSON did not report the current contract'
  echo 'ok: profile writer requires preset args to be an array of strings'
}
