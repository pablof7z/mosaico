# shellcheck shell=bash
# e2e/lib.sh — shared configuration + helpers for the mosaico end-to-end rig.
#
# Sourced by run.sh / teardown.sh. Everything here is parameterizable via env so
# the same rig can later be pointed at new features (e.g. subgroup task rooms).
#
# A "backend" is one fully isolated mosaico install: its own daemon, socket,
# state.db, agent keystore, config.json and identity key. Two backends on one
# machine talk ONLY through the local relay — there is no shared filesystem
# state between them. That is what makes this a real multi-backend test.

set -euo pipefail

# ── hermetic env: scrub the caller's live mosaico session identity ─────────
#
# The rig may be run from INSIDE a live mosaico agent session (a developer or
# an agent driving it from a hooked shell). That parent process exports its own
# session identity — MOSAICO_CHANNEL, MOSAICO_AGENT, the harness session id
# — and the auto-spawned isolated daemon inherits the client's environment
# wholesale. A leaked MOSAICO_CHANNEL is the worst offender: a bare
# `session-start` (no channel) would then scope the isolated backend into the
# CALLER's channel name, minting a spurious child group under the test workspace
# (observed: a `name=<caller-channel>` 39000 with `parent=<test-workspace>`).
#
# Unset these ONCE here, at source time, BEFORE any run-*.sh applies its own
# intentional per-call prefixes (e.g. `MOSAICO_CHANNEL=<room> mosaico ...` in
# run-ordinal.sh). Scrubbing per-call inside `mosaico()` would instead clobber those
# deliberate values — this is why the scrub lives at harness entry, not in mosaico().
# MOSAICO_HOME and MOSAICO_CONFIG are re-set per call by `mosaico()`; unsetting the
# ambient ones keeps any stray un-wrapped invocation off the caller's real home.
unset MOSAICO_CHANNEL \
      MOSAICO_AGENT \
      MOSAICO_PUBKEY \
      MOSAICO_PTY_SESSION \
      MOSAICO_PTY_SOCKET \
      CLAUDE_CODE_SESSION_ID \
      MOSAICO_HOME \
      MOSAICO_CONFIG 2>/dev/null || true

# ── tunables (override from the environment) ─────────────────────────────────

# Where the relay listens. The NIP-29 relay speaks plain ws:// (no TLS) locally.
: "${RELAY_PORT:=10547}"
: "${RELAY_HOST:=127.0.0.1}"
RELAY_WS="ws://${RELAY_HOST}:${RELAY_PORT}"
RELAY_HTTP="http://${RELAY_HOST}:${RELAY_PORT}"

# Croissant is external test infrastructure, never built or installed by Mosaico.
: "${NIP29_RELAY_BIN:=$(command -v croissant 2>/dev/null || true)}"

# The mosaico binary under test: this worktree's debug build, resolved
# relative to this file so the rig works from any cwd. Override ONLY with the
# dedicated E2E_MOSAICO_BIN — NOT $MOSAICO_BIN, which mosaico itself
# reads as the daemon-spawn override and is commonly exported in a dev shell
# (pointing at the installed binary). Defaulting to that would silently test the
# wrong binary.
E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${E2E_DIR}/.." && pwd)"
MOSAICO_BIN="${E2E_MOSAICO_BIN:-${REPO_ROOT}/target/debug/mosaico}"

# Scratch root for ALL ephemeral state (relay data, backend homes, logs, pids).
# Wiped by teardown.sh. Kept out of the repo tree.
: "${E2E_WORK:=${TMPDIR:-/tmp}/mosaico-e2e}"

# The workspace the smoke test drives. A plain slug — the daemon creates a NIP-29
# group with this as its `h`/`d` id on first session-start.
: "${E2E_WORKSPACE:=e2e-demo}"

# Turn on verbose daemon logging in both backends.
: "${MOSAICO_DEBUG:=1}"
export MOSAICO_DEBUG

# ── derived paths ────────────────────────────────────────────────────────────

RELAY_DATA="${E2E_WORK}/relay-data"
RELAY_LOG="${E2E_WORK}/relay.log"
RELAY_PIDFILE="${E2E_WORK}/relay.pid"
KEYS_DIR="${E2E_WORK}/keys"

# Per-backend home: $E2E_WORK/<name>/{config.json, mosaico/}
backend_root() { echo "${E2E_WORK}/$1"; }
backend_config() { echo "$(backend_root "$1")/config.json"; }
backend_mosaico_home() { echo "$(backend_root "$1")/mosaico"; }
backend_pidfile() { echo "$(backend_root "$1")/daemon.pid"; }
# Each backend uses an independent checkout path so workspace-root discovery does
# not collapse both installs onto one shared git root.
backend_workspace_dir() { echo "$(backend_root "$1")/work/${E2E_WORKSPACE}"; }

# ── colored logging ──────────────────────────────────────────────────────────

_c_reset=$'\033[0m'; _c_blue=$'\033[34m'; _c_green=$'\033[32m'
_c_yellow=$'\033[33m'; _c_red=$'\033[31m'; _c_dim=$'\033[2m'
log()  { printf '%s==>%s %s\n' "$_c_blue"  "$_c_reset" "$*"; }
ok()   { printf '%s ok %s %s\n' "$_c_green" "$_c_reset" "$*"; }
warn() { printf '%swarn%s %s\n' "$_c_yellow" "$_c_reset" "$*"; }
die()  { printf '%sFAIL%s %s\n' "$_c_red"   "$_c_reset" "$*" >&2; exit 1; }
dim()  { printf '%s%s%s\n' "$_c_dim" "$*" "$_c_reset"; }

# ── key helpers (nak required) ───────────────────────────────────────────────

require_nak() {
  command -v nak >/dev/null 2>&1 || die "nak (Nostr army knife) not found on PATH — install it or set up keys manually"
}

require_nip29_relay() {
  [[ -n "${NIP29_RELAY_BIN}" && -x "${NIP29_RELAY_BIN}" ]] || \
    die "Croissant executable not found — set NIP29_RELAY_BIN"
}

nak_req_contains() {
  local needle="$1"; shift
  local out
  out="$(nak_req_limited 4 "$@" 2>/dev/null || true)"
  [[ "${out}" == *"${needle}"* ]]
}

run_limited() {
  local seconds="$1"; shift
  "$@" &
  local cmd_pid=$!
  (
    sleep "${seconds}"
    terminate_tree TERM "${cmd_pid}"
    sleep 1
    terminate_tree KILL "${cmd_pid}"
  ) &
  local killer_pid=$!
  local rc=0
  wait "${cmd_pid}" 2>/dev/null || rc=$?
  kill "${killer_pid}" 2>/dev/null || true
  wait "${killer_pid}" 2>/dev/null || true
  return "${rc}"
}

child_pids() {
  local parent="$1"
  ps -axo pid=,ppid= | awk -v p="${parent}" '$2 == p { print $1 }'
}

terminate_tree() {
  local signal="$1" pid="$2" child
  for child in $(child_pids "${pid}"); do
    terminate_tree "${signal}" "${child}"
  done
  kill "-${signal}" "${pid}" 2>/dev/null || true
}

nak_req_limited() {
  local seconds="$1"; shift
  run_limited "${seconds}" nak req "$@" || true
}

# Mint a backend's identity key once and cache it under KEYS_DIR. Idempotent:
# re-running run.sh reuses the same keys so group admin membership is stable.
backend_seckey() {
  local name="$1" f="${KEYS_DIR}/$1.sk"
  if [[ ! -s "$f" ]]; then
    mkdir -p "${KEYS_DIR}"
    nak key generate >"$f"
  fi
  cat "$f"
}
backend_pubkey() { nak key public "$(backend_seckey "$1")"; }

# ── mosaico invocation with a backend's isolated environment ──────────────

# Run the mosaico binary as backend <name>: points MOSAICO_CONFIG and
# MOSAICO_HOME at that backend's private tree. The daemon, when auto-spawned
# by this client call, inherits exactly these vars (Command keeps the env), so
# the backend's daemon is bound to the same isolated home.
mosaico() {
  local name="$1"; shift
  # `env -u MOSAICO_BIN`: scrub any inherited daemon-spawn override so the
  # auto-spawned daemon re-execs THIS binary via current_exe(), not whatever the
  # dev shell exported. The client process is ${MOSAICO_BIN} explicitly.
  env -u MOSAICO_BIN \
    MOSAICO_CONFIG="$(backend_config "$name")" \
    MOSAICO_HOME="$(backend_mosaico_home "$name")" \
    MOSAICO_DEBUG="${MOSAICO_DEBUG}" \
    "${MOSAICO_BIN}" "$@"
}

record_backend_daemon_pid() {
  local name="$1" pid
  pid="$(pgrep -n -f "${MOSAICO_BIN} daemon" || true)"
  [[ -n "${pid}" ]] && echo "${pid}" >"$(backend_pidfile "${name}")"
}

# Hooks deliberately never spawn a daemon. Start the isolated backend through a
# normal client RPC before driving hook input, then record its exact daemon PID
# for teardown.
start_backend_daemon() {
  local name="$1"
  mosaico "${name}" channel list --all-workspaces >/dev/null
  record_backend_daemon_pid "${name}"
}

# ── relay liveness ───────────────────────────────────────────────────────────

relay_up() {
  # NIP-11: a GET with the nostr+json Accept header returns the relay info doc.
  curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" 2>/dev/null \
    | grep -q '"supported_nips"'
}

wait_for() {
  # wait_for <desc> <timeout-secs> <shell-snippet>
  # The snippet is eval'd in THIS shell each poll, so it can use the `mosaico`
  # helper function (which a `bash -c` child shell would not inherit). Returns 0
  # as soon as the snippet succeeds; dies on timeout.
  local desc="$1" timeout="$2" snippet="$3"
  local deadline=$(( $(date +%s) + timeout ))
  while (( $(date +%s) < deadline )); do
    if eval "${snippet}" >/dev/null 2>&1; then return 0; fi
    sleep 0.3
  done
  die "timed out after ${timeout}s waiting for: ${desc}"
}

# Like wait_for but returns 1 on timeout instead of dying (caller decides).
wait_for_soft() {
  local desc="$1" timeout="$2" snippet="$3"
  local deadline=$(( $(date +%s) + timeout ))
  while (( $(date +%s) < deadline )); do
    if eval "${snippet}" >/dev/null 2>&1; then return 0; fi
    sleep 0.3
  done
  return 1
}
