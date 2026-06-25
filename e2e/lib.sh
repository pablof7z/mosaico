# shellcheck shell=bash
# e2e/lib.sh — shared configuration + helpers for the tenex-edge end-to-end rig.
#
# Sourced by run.sh / teardown.sh. Everything here is parameterizable via env so
# the same rig can later be pointed at new features (e.g. subgroup task rooms).
#
# A "backend" is one fully isolated tenex-edge install: its own daemon, socket,
# state.db, agent keystore, config.json and identity key. Two backends on one
# machine talk ONLY through the local relay — there is no shared filesystem
# state between them. That is what makes this a real multi-backend test.

set -euo pipefail

# ── tunables (override from the environment) ─────────────────────────────────

# Where the relay listens. The NIP-29 relay speaks plain ws:// (no TLS) locally.
: "${RELAY_PORT:=10547}"
: "${RELAY_HOST:=127.0.0.1}"
RELAY_WS="ws://${RELAY_HOST}:${RELAY_PORT}"
RELAY_HTTP="http://${RELAY_HOST}:${RELAY_PORT}"

# NIP-29 relay source checkout + built binary. Build is done once by run.sh.
: "${NIP29_RELAY_DIR:=/Users/pablofernandez/Work/croissant}"
: "${NIP29_RELAY_BIN:=${NIP29_RELAY_DIR}/croissant}"

# The tenex-edge binary under test: this worktree's debug build, resolved
# relative to this file so the rig works from any cwd. Override ONLY with the
# dedicated E2E_TENEX_EDGE_BIN — NOT $TENEX_EDGE_BIN, which tenex-edge itself
# reads as the daemon-spawn override and is commonly exported in a dev shell
# (pointing at the installed binary). Defaulting to that would silently test the
# wrong binary.
E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${E2E_DIR}/.." && pwd)"
TENEX_EDGE_BIN="${E2E_TENEX_EDGE_BIN:-${REPO_ROOT}/target/debug/tenex-edge}"

# Scratch root for ALL ephemeral state (relay data, backend homes, logs, pids).
# Wiped by teardown.sh. Kept out of the repo tree.
: "${E2E_WORK:=${TMPDIR:-/tmp}/tenex-edge-e2e}"

# The project the smoke test drives. A plain slug — the daemon creates a NIP-29
# group with this as its `h`/`d` id on first session-start.
: "${E2E_PROJECT:=e2e-demo}"

# Turn on verbose daemon logging in both backends.
: "${TENEX_EDGE_DEBUG:=1}"
export TENEX_EDGE_DEBUG

# ── derived paths ────────────────────────────────────────────────────────────

RELAY_DATA="${E2E_WORK}/relay-data"
RELAY_LOG="${E2E_WORK}/relay.log"
RELAY_PIDFILE="${E2E_WORK}/relay.pid"
KEYS_DIR="${E2E_WORK}/keys"

# Per-backend home: $E2E_WORK/<name>/{config.json, edge/, tenex/}
backend_root() { echo "${E2E_WORK}/$1"; }
backend_config() { echo "$(backend_root "$1")/config.json"; }
backend_edge_home() { echo "$(backend_root "$1")/edge"; }
backend_tenex_dir() { echo "$(backend_root "$1")/tenex"; }
backend_pidfile() { echo "$(backend_root "$1")/daemon.pid"; }
# Project working dir lives under the backend's tenex dir so the two backends
# resolve the SAME project slug from backend-local project maps while keeping
# independent checkouts (no shared git root that could collapse the slug).
backend_project_dir() { echo "$(backend_root "$1")/work/${E2E_PROJECT}"; }

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

nak_req_contains() {
  local needle="$1"; shift
  local out
  out="$(nak req "$@" 2>/dev/null || true)"
  [[ "${out}" == *"${needle}"* ]]
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

# ── tenex-edge invocation with a backend's isolated environment ──────────────

# Run the tenex-edge binary as backend <name>: points TENEX_CONFIG / TENEX_DIR /
# TENEX_EDGE_HOME at that backend's private tree. The daemon, when auto-spawned
# by this client call, inherits exactly these vars (Command keeps the env), so
# the backend's daemon is bound to the same isolated home.
edge() {
  local name="$1"; shift
  # `env -u TENEX_EDGE_BIN`: scrub any inherited daemon-spawn override so the
  # auto-spawned daemon re-execs THIS binary via current_exe(), not whatever the
  # dev shell exported. The client process is ${TENEX_EDGE_BIN} explicitly.
  env -u TENEX_EDGE_BIN \
    TENEX_CONFIG="$(backend_config "$name")" \
    TENEX_DIR="$(backend_tenex_dir "$name")" \
    TENEX_EDGE_HOME="$(backend_edge_home "$name")" \
    TENEX_EDGE_DEBUG="${TENEX_EDGE_DEBUG}" \
    "${TENEX_EDGE_BIN}" "$@"
}

# ── relay liveness ───────────────────────────────────────────────────────────

relay_up() {
  # NIP-11: a GET with the nostr+json Accept header returns the relay info doc.
  curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" 2>/dev/null \
    | grep -q '"supported_nips"'
}

wait_for() {
  # wait_for <desc> <timeout-secs> <shell-snippet>
  # The snippet is eval'd in THIS shell each poll, so it can use the `edge`
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
