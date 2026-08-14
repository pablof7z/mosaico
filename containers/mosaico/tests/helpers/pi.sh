setup_pi_host_fixture() {
  mkdir -p "${HOST_HOME}/.pi/agent"
  printf '{"openai-codex":{}}\n' >"${HOST_HOME}/.pi/agent/auth.json"
  printf '{"defaultProvider":"openai-codex"}\n' \
    >"${HOST_HOME}/.pi/agent/settings.json"
}

assert_pi_state_isolated() {
  AGENT=pi
  STATE_DIR="${TMP}/state-pi"
  mkdir -p "${STATE_DIR}/home/.pi/agent"
  stage_host_auth
  [[ -L "${STATE_DIR}/home/.pi/agent/auth.json" ]] \
    || fail 'Pi auth was not staged through its provider-scoped mount'
  [[ "$(readlink "${STATE_DIR}/home/.pi/agent/auth.json")" == /host-auth/pi/auth.json ]] \
    || fail 'Pi auth points at the wrong provider-scoped path'
  cmp -s "${HOST_HOME}/.pi/agent/settings.json" \
    "${STATE_DIR}/home/.pi/agent/settings.json" \
    || fail 'Pi settings were not copied into isolated state'
  echo 'ok: Pi auth and settings are staged into isolated state'
}
