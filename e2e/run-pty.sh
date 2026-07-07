#!/usr/bin/env bash
# e2e/run-pty.sh — portable PTY client/supervisor controls without model auth.
#
# This rig starts the hidden PTY supervisor around a harmless `cat` process,
# registers matching metadata in an isolated TENEX_EDGE_HOME, then exercises the
# public client commands that real launched agents depend on:
#   - pty list / liveness
#   - attach protocol backlog/fanout through a Unix-socket client
#   - bracketed multi-line inject from stdin plus explicit submit
#   - plain multi-line inject
#   - resize command acceptance
#   - kill and metadata cleanup

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

command -v nc >/dev/null 2>&1 || die "nc not found on PATH — required for Unix-socket attach probe"

PASS_N=0
FAIL_N=0
declare -a RESULTS=()
check_pass() { PASS_N=$((PASS_N + 1)); RESULTS+=("PASS  $1"); printf '%s PASS %s %s\n' "$_c_green" "$_c_reset" "$1"; }
check_fail() { FAIL_N=$((FAIL_N + 1)); RESULTS+=("FAIL  $1"); printf '%sFAIL%s %s\n' "$_c_red" "$_c_reset" "$1" >&2; }

SUP_PID=""
NC_PID=""
FIFO_FD_OPEN=0
cleanup() {
  if [[ "${FIFO_FD_OPEN}" == "1" ]]; then exec 3>&- || true; fi
  [[ -n "${NC_PID}" ]] && kill "${NC_PID}" 2>/dev/null || true
  [[ -n "${SUP_PID}" ]] && kill "${SUP_PID}" 2>/dev/null || true
  [[ -n "${SUP_PID}" ]] && kill -9 "${SUP_PID}" 2>/dev/null || true
}
trap cleanup EXIT

log "building tenex-edge under test (cargo build)"
( cd "${REPO_ROOT}" && cargo build ) || die "cargo build failed"
[[ -x "${TENEX_EDGE_BIN}" ]] || die "tenex-edge binary not found at ${TENEX_EDGE_BIN}"

log "step 0: tearing down any previous run"
E2E_KEEP_DATA=0 "${E2E_DIR}/teardown.sh" >/dev/null 2>&1 || true
mkdir -p "${E2E_WORK}" "${KEYS_DIR}"

ID="pty-probe-$$"
A_EDGE="$(backend_edge_home edge-a)"
A_TDIR="$(backend_tenex_dir edge-a)"
A_CFG="$(backend_config edge-a)"
PTY_DIR="${A_EDGE}/pty"
PTY_WORK="${E2E_WORK}/pty-work"
SOCKET="${PTY_DIR}/${ID}.sock"
CAPTURE="${E2E_WORK}/pty-capture.log"
LAUNCH_CAPTURE="${E2E_WORK}/pty-launch-capture.log"
LAUNCH_OUT="${E2E_WORK}/pty-launch.out"
ATTACH_LOG="${E2E_WORK}/pty-attach.log"
ATTACH_ERR="${E2E_WORK}/pty-attach.err"
SUP_OUT="${E2E_WORK}/pty-supervisor.out"
SUP_ERR="${E2E_WORK}/pty-supervisor.err"
FIFO="${E2E_WORK}/pty-attach.in"
mkdir -p "${A_EDGE}" "${A_TDIR}" "${PTY_DIR}" "${PTY_WORK}"

cat >"${A_CFG}" <<JSON
{
  "whitelistedPubkeys": [],
  "relays": [],
  "indexerRelay": "",
  "backendName": "edge-a"
}
JSON

log "step 1: starting portable PTY supervisor (${ID})"
env -u TENEX_EDGE_BIN \
  TENEX_CONFIG="${A_CFG}" \
  TENEX_DIR="${A_TDIR}" \
  TENEX_EDGE_HOME="${A_EDGE}" \
  PTY_CAPTURE="${CAPTURE}" \
  "${TENEX_EDGE_BIN}" __pty-supervisor \
    --id "${ID}" \
    --socket "${SOCKET}" \
    --cwd "${PTY_WORK}" \
    --agent "pty-probe" \
    -- sh -lc 'printf "pty-ready\n"; cat > "$PTY_CAPTURE"' \
    >"${SUP_OUT}" 2>"${SUP_ERR}" &
SUP_PID=$!

cat >"${PTY_DIR}/${ID}.json" <<JSON
{
  "id": "${ID}",
  "socket": "${SOCKET}",
  "supervisor_pid": ${SUP_PID},
  "agent": "pty-probe",
  "project": "pty-probe",
  "cwd": "${PTY_WORK}",
  "command": ["sh", "-lc", "cat"]
}
JSON

wait_for "PTY session to be live" 10 "edge edge-a pty list 2>/dev/null | grep -q '${ID}.*yes'"
check_pass "1 pty list — metadata resolves to a live supervisor"

log "step 2: attaching a socket client"
mkfifo "${FIFO}"
nc -U "${SOCKET}" <"${FIFO}" >"${ATTACH_LOG}" 2>"${ATTACH_ERR}" &
NC_PID=$!
exec 3>"${FIFO}"
FIFO_FD_OPEN=1
printf 'ATTACH 24 80\n' >&3

if wait_for_soft "attach backlog to include ready marker" 8 \
     "LC_ALL=C grep -Fq 'pty-ready' '${ATTACH_LOG}'"; then
  check_pass "2 attach — client receives PTY backlog"
else
  check_fail "2 attach — ready marker missing from attach log"
fi

contains_capture() { LC_ALL=C grep -Fq "$1" "${CAPTURE}"; }
contains_attach() { LC_ALL=C grep -Fq "$1" "${ATTACH_LOG}"; }
OPEN="$(printf '\033[200~')"
CLOSE="$(printf '\033[201~')"

log "step 3: bracketed multi-line inject from stdin"
printf 'alpha line\nbeta line' | edge edge-a pty inject --bracketed --no-submit "${ID}"
edge edge-a pty inject "${ID}" ""
if wait_for_soft "capture bracketed multi-line paste" 8 \
     "contains_capture \"${OPEN}alpha line\" && contains_capture 'beta line' && contains_capture \"${CLOSE}\""; then
  check_pass "3 bracketed inject — multi-line paste reached the PTY with bracket markers"
else
  check_fail "3 bracketed inject — capture did not contain the bracketed multi-line payload"
fi

log "step 4: plain multi-line inject from stdin"
printf 'gamma line\ndelta line' | edge edge-a pty inject --no-submit "${ID}"
edge edge-a pty inject "${ID}" ""
if wait_for_soft "capture plain multi-line paste" 8 \
     "contains_capture 'gamma line' && contains_capture 'delta line'"; then
  check_pass "4 plain inject — multi-line payload reached the PTY"
else
  check_fail "4 plain inject — capture did not contain the plain multi-line payload"
fi

if wait_for_soft "attach fanout to include injected text" 8 \
     "contains_attach 'gamma line' && contains_attach 'delta line'"; then
  check_pass "5 attach fanout — attached client sees live PTY echo"
else
  check_fail "5 attach fanout — attached client did not see injected text"
fi

log "step 5: resize and kill"
if edge edge-a pty resize --rows 33 --cols 100 "${ID}"; then
  check_pass "6 resize — command accepted by supervisor"
else
  check_fail "6 resize — command failed"
fi

edge edge-a pty kill "${ID}" || check_fail "7 kill — command failed"
exec 3>&- || true
FIFO_FD_OPEN=0
kill "${NC_PID}" 2>/dev/null || true
wait "${NC_PID}" 2>/dev/null || true
NC_PID=""

if wait_for_soft "supervisor exits and metadata is removed" 10 \
     "! kill -0 '${SUP_PID}' 2>/dev/null && ! test -e '${PTY_DIR}/${ID}.json'"; then
  SUP_PID=""
  check_pass "7 kill — supervisor exited and metadata was removed"
else
  check_fail "7 kill — supervisor or metadata remained live"
fi

log "step 6: tenex-edge launch <agent> uses portable PTY mode"
(
  cd "${PTY_WORK}"
  edge edge-a agent add launch-probe -- \
    env "LAUNCH_CAPTURE=${LAUNCH_CAPTURE}" /bin/sh -lc \
    'printf "launch-ready\n"; cat > "$LAUNCH_CAPTURE"' >/dev/null
  edge edge-a launch launch-probe --project pty-probe >"${LAUNCH_OUT}" 2>&1
) || check_fail "8 launch — tenex-edge launch command failed"
LAUNCH_ID="$(sed -n 's/.*session: //p' "${LAUNCH_OUT}" | tail -1 | tr -d '\r')"
if [[ -n "${LAUNCH_ID}" ]] && wait_for_soft "launched PTY session to be live" 10 \
     "edge edge-a pty list 2>/dev/null | grep -q '${LAUNCH_ID}.*yes'"; then
  check_pass "8 launch — tenex-edge launch created a live portable PTY session"
else
  check_fail "8 launch — live PTY session not found after launch"
fi

if [[ -n "${LAUNCH_ID}" ]]; then
  printf 'launch input line' | edge edge-a pty inject --no-submit "${LAUNCH_ID}"
  edge edge-a pty inject "${LAUNCH_ID}" ""
  if wait_for_soft "launch capture to include injected input" 8 \
       "LC_ALL=C grep -Fq 'launch input line' '${LAUNCH_CAPTURE}'"; then
    check_pass "9 launch inject — launched PTY receives programmatic input"
  else
    check_fail "9 launch inject — launched PTY did not receive injected input"
  fi
  edge edge-a pty kill "${LAUNCH_ID}" || check_fail "10 launch kill — command failed"
  if wait_for_soft "launched PTY metadata is removed" 10 \
       "! test -e '${PTY_DIR}/${LAUNCH_ID}.json'"; then
    check_pass "10 launch kill — launched PTY metadata was removed"
  else
    check_fail "10 launch kill — launched PTY metadata remained"
  fi
fi

echo
log "portable PTY e2e summary"
for line in "${RESULTS[@]}"; do
  case "${line}" in
    PASS*) printf '  %s%s%s\n' "$_c_green" "${line}" "$_c_reset" ;;
    FAIL*) printf '  %s%s%s\n' "$_c_red" "${line}" "$_c_reset" ;;
  esac
done
printf 'totals: %sPASS=%d%s  %sFAIL=%d%s\n' \
  "$_c_green" "${PASS_N}" "$_c_reset" \
  "$_c_red" "${FAIL_N}" "$_c_reset"

cat <<NOTE

capture    ${CAPTURE}
launch     ${LAUNCH_CAPTURE}
attach log ${ATTACH_LOG}
supervisor ${SUP_ERR}
tear down  ./e2e/teardown.sh
NOTE

if [[ "${FAIL_N}" -gt 0 ]]; then
  die "${FAIL_N} hard check(s) FAILED"
fi
ok "no hard failures (${PASS_N} pass)"
