#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
SCRIPT="${ROOT}/scripts/install-fleet"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_contains() {
  local needle="$1" path="$2" label="$3"
  grep -Fq -- "${needle}" "${path}" || fail "${label}: missing [${needle}]"
  echo "ok: ${label}"
}

ORIGIN="${TMP}/origin.git"
SEED="${TMP}/seed"
git init --bare --initial-branch=master "${ORIGIN}" >/dev/null
git init --initial-branch=master "${SEED}" >/dev/null
git -C "${SEED}" config user.email test@example.com
git -C "${SEED}" config user.name 'Mosaico Test'
git -C "${SEED}" remote add origin "${ORIGIN}"
mkdir -p "${SEED}/scripts" "${SEED}/skills/mosaico" \
  "${SEED}/skills/mosaico-dev/references"
cp "${SCRIPT}" "${SEED}/scripts/install-fleet"
chmod +x "${SEED}/scripts/install-fleet"
printf '%s\n' '---' 'name: mosaico' '---' >"${SEED}/skills/mosaico/SKILL.md"
printf '%s\n' '---' 'name: mosaico-dev' '---' \
  >"${SEED}/skills/mosaico-dev/SKILL.md"
printf 'current grok lab\n' \
  >"${SEED}/skills/mosaico-dev/references/grok-pty-lab.md"
printf '/target\n' >"${SEED}/.gitignore"
git -C "${SEED}" add .
git -C "${SEED}" commit -m initial >/dev/null
git -C "${SEED}" push -u origin master >/dev/null

LOCAL_REPO="${TMP}/local"
REMOTE_REPO="${TMP}/remote"
git clone "${ORIGIN}" "${LOCAL_REPO}" >/dev/null
git clone "${ORIGIN}" "${REMOTE_REPO}" >/dev/null
printf 'origin advanced\n' >"${SEED}/CURRENT"
git -C "${SEED}" add CURRENT
git -C "${SEED}" commit -m current >/dev/null
git -C "${SEED}" push origin master >/dev/null
EXPECTED="$(git -C "${SEED}" rev-parse HEAD)"

TEST_HOME="${TMP}/home"
FAKE_BIN="${TEST_HOME}/.cargo/bin"
LOG="${TMP}/commands.log"
mkdir -p "${FAKE_BIN}" "${TEST_HOME}/.claude" "${TEST_HOME}/.local/bin" \
  "${TEST_HOME}/.agents/skills/mosaico-dev"
printf 'stale copied skill\n' \
  >"${TEST_HOME}/.agents/skills/mosaico-dev/STALE"

cat >"${FAKE_BIN}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-} ${2:-}" == 'build --release' ]]
printf 'cargo build --release %s\n' "${PWD}" >>"${FLEET_TEST_LOG}"
mkdir -p target/release
cp "${FLEET_FAKE_MOSAICO}" target/release/mosaico
chmod +x target/release/mosaico
EOF

cat >"${FAKE_BIN}/mosaico-fake" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'mosaico %s\n' "$*" >>"${FLEET_TEST_LOG}"
case "${1:-}" in
  install)
    if [[ "${2:-}" == --all ]]; then
      mkdir -p "${HOME}/.agents/skills" "${HOME}/.claude/skills"
      rm -rf "${HOME}/.agents/skills/mosaico" \
        "${HOME}/.claude/skills/mosaico"
      ln -s "${PWD}/skills/mosaico" "${HOME}/.agents/skills/mosaico"
      ln -s "${HOME}/.agents/skills/mosaico" \
        "${HOME}/.claude/skills/mosaico"
    else
      echo 'fake install status'
    fi
    ;;
  daemon)
    [[ "${2:-}" == restart ]]
    ;;
  --version)
    echo 'mosaico test'
    ;;
  *) exit 2 ;;
esac
EOF

cat >"${FAKE_BIN}/ssh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'ssh %s\n' "$1" >>"${FLEET_TEST_LOG}"
shift
exec "$@"
EOF

cat >"${FAKE_BIN}/uname" <<'EOF'
#!/usr/bin/env bash
echo Linux
EOF
chmod +x "${FAKE_BIN}"/*

OUTPUT="${TMP}/output"
HOME="${TEST_HOME}" \
PATH="${FAKE_BIN}:${PATH}" \
FLEET_FAKE_MOSAICO="${FAKE_BIN}/mosaico-fake" \
FLEET_TEST_LOG="${LOG}" \
MOSAICO_FLEET_LOCAL_REPO="${LOCAL_REPO}" \
  bash "${SCRIPT}" fake-host="${REMOTE_REPO}" >"${OUTPUT}" 2>&1

[[ "$(git -C "${LOCAL_REPO}" rev-parse HEAD)" == "${EXPECTED}" ]] \
  || fail 'local checkout was not updated to origin/master'
[[ "$(git -C "${REMOTE_REPO}" rev-parse HEAD)" == "${EXPECTED}" ]] \
  || fail 'remote checkout was not updated to origin/master'
[[ "$(cd "${TEST_HOME}/.agents/skills/mosaico-dev" && pwd -P)" \
  == "$(cd "${REMOTE_REPO}/skills/mosaico-dev" && pwd -P)" ]] \
  || fail 'mosaico-dev skill did not resolve to the current checkout'
[[ -L "${TEST_HOME}/.agents/skills/mosaico-dev" ]] \
  || fail 'stale copied development skill was not replaced with a symlink'
[[ -f "${TEST_HOME}/.agents/skills/mosaico-dev/references/grok-pty-lab.md" ]] \
  || fail 'installed development skill is incomplete'
assert_contains 'fleet verified: local + 1 remote host(s)' "${OUTPUT}" \
  'fleet success summary'
assert_contains 'binary:' "${OUTPUT}" 'installed binary hash is reported'
[[ "$(grep -Fc 'cargo build --release' "${LOG}")" -eq 2 ]] \
  || fail 'release binary was not built exactly once per host'
echo 'ok: portable release build ran once per host'
assert_contains 'ssh fake-host' "${LOG}" 'remote worker was streamed over SSH'
[[ "$(grep -Fc 'mosaico daemon restart' "${LOG}")" -eq 2 ]] \
  || fail 'daemon was not restarted exactly once per host'
echo 'ok: daemon restarted once per host'

printf 'dirty\n' >"${LOCAL_REPO}/DIRTY"
set +e
HOME="${TEST_HOME}" \
PATH="${FAKE_BIN}:${PATH}" \
FLEET_FAKE_MOSAICO="${FAKE_BIN}/mosaico-fake" \
FLEET_TEST_LOG="${LOG}" \
MOSAICO_FLEET_LOCAL_REPO="${LOCAL_REPO}" \
  bash "${SCRIPT}" >"${TMP}/dirty-output" 2>&1
DIRTY_STATUS=$?
set -e
[[ "${DIRTY_STATUS}" -ne 0 ]] || fail 'dirty checkout unexpectedly passed'
assert_contains 'has local changes' "${TMP}/dirty-output" \
  'dirty checkout failure is actionable'

rm "${LOCAL_REPO}/DIRTY"
git -C "${LOCAL_REPO}" switch -c topic >/dev/null
set +e
HOME="${TEST_HOME}" \
PATH="${FAKE_BIN}:${PATH}" \
FLEET_FAKE_MOSAICO="${FAKE_BIN}/mosaico-fake" \
FLEET_TEST_LOG="${LOG}" \
MOSAICO_FLEET_LOCAL_REPO="${LOCAL_REPO}" \
  bash "${SCRIPT}" >"${TMP}/branch-output" 2>&1
BRANCH_STATUS=$?
set -e
[[ "${BRANCH_STATUS}" -ne 0 ]] || fail 'non-master checkout unexpectedly passed'
assert_contains "is on 'topic', expected master" "${TMP}/branch-output" \
  'non-master checkout failure is actionable'
