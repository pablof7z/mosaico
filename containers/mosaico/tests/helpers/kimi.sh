setup_kimi_host_fixture() {
  mkdir -p \
    "${HOST_HOME}/.agents/agents" \
    "${HOST_HOME}/.kimi-code/agents" \
    "${HOST_HOME}/.kimi-code/credentials"
  printf 'default_model = "test"\n' >"${HOST_HOME}/.kimi-code/config.toml"
  printf '{"access_token":"test"}\n' \
    >"${HOST_HOME}/.kimi-code/credentials/kimi-code.json"
  printf 'device-test\n' >"${HOST_HOME}/.kimi-code/device_id"
  printf '%s\n' '---' 'name: brand-reviewer' 'description: Brand reviewer' '---' \
    >"${HOST_HOME}/.kimi-code/agents/brand-reviewer.md"
  printf '%s\n' '---' 'name: shared-reviewer' 'description: Shared reviewer' '---' \
    >"${HOST_HOME}/.agents/agents/shared-reviewer.md"
}

assert_kimi_state_isolated() {
  AGENT=kimi
  STATE_DIR="${TMP}/state-kimi"
  mkdir -p "${STATE_DIR}/home/.kimi-code/credentials"
  stage_kimi_state
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
  echo 'ok: Kimi config and native profiles are staged without rotating OAuth credentials'
}

assert_kimi_login_runner() {
  local state="${TMP}/kimi-login-state"
  local args="${TMP}/kimi-login-args"
  PATH="${TMP}/fake-bin:${PATH}" \
    MOSAICO_CONTAINER_HOST_AUTH=1 \
    MOSAICO_CONTAINER_STATE="${state}" \
    FAKE_CONTAINER_ARGS_FILE="${args}" \
    bash "${ROOT}/containers/mosaico/run" kimi-login
  grep -Fxq 'kimi' "${args}" \
    || fail 'kimi-login did not invoke the Kimi CLI'
  grep -Fxq 'login' "${args}" \
    || fail 'kimi-login did not request profile-local authentication'
  if grep -Fq '/host-auth/kimi' "${args}"; then
    fail 'kimi-login unexpectedly mounted rotating host OAuth credentials'
  fi
  echo 'ok: kimi-login authenticates only the durable isolated profile'
}
