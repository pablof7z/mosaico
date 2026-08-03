test_kimi_launch() {
  local state="${TMP}/kimi-acp-state" env_file="${TMP}/kimi-acp.env"
  local output tail
  write_profile "${state}" kimi kimi-acp acp
  write_lab_env "${env_file}" "${state}"
  output="$(
    PATH="${TMP}/launcher-bin:${PATH}" \
      MOSAICO_DEV_PROMPT='inspect kimi identity' \
      bash "${SKILL}/scripts/launch-agent" "${env_file}" launch kimi-acp
  )"
  tail="$(printf '%s\n' "${output}" | launch_tail | sed -n '1,2p')"
  assert_eq $'<kimi>\n<inspect kimi identity>' "${tail}" \
    'Kimi ACP launch uses the hosted launch contract'
}

test_kimi_host_auth() {
  mkdir -p \
    "${HOST_HOME}/.agents/agents" \
    "${HOST_HOME}/.kimi-code/agents" \
    "${HOST_HOME}/.kimi-code/credentials" \
    "${STATE_DIR}/home/.kimi-code"
  printf 'default_model = "test"\n' >"${HOST_HOME}/.kimi-code/config.toml"
  printf '{"access_token":"test"}\n' \
    >"${HOST_HOME}/.kimi-code/credentials/kimi-code.json"
  printf 'device-test\n' >"${HOST_HOME}/.kimi-code/device_id"
  printf '%s\n' '---' 'name: brand-reviewer' 'description: Brand reviewer' '---' \
    >"${HOST_HOME}/.kimi-code/agents/brand-reviewer.md"
  printf '%s\n' '---' 'name: shared-reviewer' 'description: Shared reviewer' '---' \
    >"${HOST_HOME}/.agents/agents/shared-reviewer.md"
  export AGENT=kimi
  stage_kimi_state
  build_host_auth_mounts
  cmp -s "${HOST_HOME}/.kimi-code/config.toml" \
    "${STATE_DIR}/home/.kimi-code/config.toml" \
    || fail 'Kimi config was not copied into isolated state'
  printf 'profile_local = true\n' >>"${STATE_DIR}/home/.kimi-code/config.toml"
  stage_kimi_state
  grep -Fq 'profile_local = true' "${STATE_DIR}/home/.kimi-code/config.toml" \
    || fail 'Kimi profile-local config was overwritten after initial seeding'
  [[ ! -e "${STATE_DIR}/home/.kimi-code/credentials/kimi-code.json" ]] \
    || fail 'Kimi rotating OAuth credentials must not be copied from the host'
  for path in \
    .kimi-code/agents/brand-reviewer.md \
    .agents/agents/shared-reviewer.md; do
    cmp -s "${HOST_HOME}/${path}" "${STATE_DIR}/home/${path}" \
      || fail "Kimi native profile ${path} was not copied"
    [[ ! -L "${STATE_DIR}/home/${path}" ]] \
      || fail "Kimi native profile ${path} must not point back into host state"
  done
  [[ "${#HOST_AUTH_MOUNTS[@]}" -eq 0 ]] \
    || fail 'Kimi host auth unexpectedly exposed a host bind mount'
  echo 'ok: host auth copies Kimi config and profiles without rotating OAuth credentials'
}
