#!/usr/bin/env bash
# e2e/run-warm.sh — proactive kind:0 profile warming + backend roster exclusion
# against the REAL croissant NIP-29 relay.
#
# This is a SELF-CONTAINED rig (like run-ordinal.sh): it sources e2e/lib.sh for
# the shared helpers and reuses e2e/teardown.sh for cleanup, but boots its OWN
# relay + a single backend so it does not depend on e2e/run.sh having run first.
# It never modifies run.sh / run-ordinal.sh / run-subgroup.sh / lib.sh.
#
# WHY THIS EXISTS
#   mosaico resolves a pubkey to a human name from its kind:0, fetched from the
#   configured `indexerRelay` (production default purplepag.es; the rig points it
#   at the LOCAL relay for hermeticity — same code path, closed world). The daemon
#   warms profiles PROACTIVELY — at startup for known identities, and on every
#   inbound event for any newly-seen pubkey — so `who` renders names from cache
#   without doing any fetching itself. This rig proves that end to end.
#
# WHAT IT VERIFIES
#   1. Warming — a peer whose kind:0 lives ONLY on the relay, added to the workspace
#      as a member (relay-signed kind:39002), is resolved to its display NAME in
#      the backend's `who` roster — proving the daemon fetched + cached its kind:0
#      off the inbound membership event, with no explicit `who` warm.
#   2. Backend exclusion — the daemon's own management pubkey (a channel admin)
#      is NOT rendered as a roster member.
#
# DEGRADATION CONTRACT: only INFRASTRUCTURE problems are hard failures (relay
# down, binary missing, backend won't boot, workspace group never created, the peer
# never lands as a relay member). The two behavioral invariants above are fully
# wired, so they are hard failures. The run exits nonzero iff a hard check FAILS.
#
# Tunables: see e2e/lib.sh. Extra knobs:
#   E2E_WARM_WORKSPACE  workspace/room slug (default: warm-demo)
#   E2E_WARM_NAME     the peer's kind:0 display name (default: alice-peer)

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

# Isolate from the other rigs' default workspace slug.
E2E_WORKSPACE="${E2E_WARM_WORKSPACE:-warm-demo}"
PEER_NAME="${E2E_WARM_NAME:-alice-peer}"

require_nak
require_nip29_relay
command -v jq >/dev/null 2>&1 || die "jq not found on PATH — required to parse relay JSON"

# ── check accounting ─────────────────────────────────────────────────────────
PASS_N=0
FAIL_N=0
declare -a RESULTS=()
check_pass() { PASS_N=$((PASS_N + 1)); RESULTS+=("PASS  $1"); printf '%s PASS %s %s\n' "$_c_green" "$_c_reset" "$1"; }
check_fail() { FAIL_N=$((FAIL_N + 1)); RESULTS+=("FAIL  $1"); printf '%sFAIL%s %s\n' "$_c_red" "$_c_reset" "$1" >&2; }

# ── keepalive bookkeeping ────────────────────────────────────────────────────
KEEPALIVE_PID=""
WATCH_PIDS=()
cleanup() {
  [[ -n "${KEEPALIVE_PID}" ]] && kill "${KEEPALIVE_PID}" 2>/dev/null || true
  for wp in "${WATCH_PIDS[@]:-}"; do [[ -n "$wp" ]] && kill "$wp" 2>/dev/null || true; done
}
trap cleanup EXIT

# ── build the binary under test ──────────────────────────────────────────────
log "building mosaico under test (cargo build)"
( cd "${REPO_ROOT}" && cargo build ) || die "cargo build failed"
[[ -x "${MOSAICO_BIN}" ]] || die "mosaico binary not found at ${MOSAICO_BIN} after build"
ok "binary: ${MOSAICO_BIN}"

# ── 0. clean slate (reuse teardown.sh) ───────────────────────────────────────
log "step 0: tearing down any previous run"
E2E_KEEP_DATA=0 "${E2E_DIR}/teardown.sh" >/dev/null 2>&1 || true
mkdir -p "${E2E_WORK}" "${KEYS_DIR}"

# ── 1. boot the NIP-29 relay (croissant) ─────────────────────────────────────
log "step 1: NIP-29 relay"
ok "NIP-29 relay binary: ${NIP29_RELAY_BIN}"

mkdir -p "${RELAY_DATA}"
OWNER_PK="$(backend_pubkey mosaico-a)"
log "starting relay on ${RELAY_WS} (data: ${RELAY_DATA})"
nohup env PORT="${RELAY_PORT}" HOST="${RELAY_HOST}" DATAPATH="${RELAY_DATA}" \
    OWNER_PUBLIC_KEY="${OWNER_PK}" DOMAIN="" \
    "${NIP29_RELAY_BIN}" >"${RELAY_LOG}" 2>&1 &
RELAY_PID=$!
echo "${RELAY_PID}" >"${RELAY_PIDFILE}"
wait_for "relay NIP-11 to report supported_nips" 20 relay_up

# Stale-relay guard: the process on the port MUST be the one we just launched.
LISTENER_PID="$(lsof -nP -tiTCP:"${RELAY_PORT}" -sTCP:LISTEN 2>/dev/null | head -1 || true)"
if [[ -n "${LISTENER_PID}" && "${LISTENER_PID}" != "${RELAY_PID}" ]]; then
  die "port ${RELAY_PORT} held by pid ${LISTENER_PID}, not our relay (pid ${RELAY_PID}) — run ./e2e/teardown.sh and retry."
fi
if curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" | grep -Eq '"supported_nips":[^]]*\b29\b'; then
  ok "relay up; NIP-11 advertises NIP-29"
else
  die "relay does not advertise NIP-29 (see ${RELAY_LOG})"
fi

# ── 2. single backend config + workspace registration ──────────────────────────
log "step 2: single isolated backend (mosaico-a)"
A_SK="$(backend_seckey mosaico-a)"
A_PK="$(backend_pubkey mosaico-a)"
A_HOME="$(backend_mosaico_home mosaico-a)"
A_CFG="$(backend_config mosaico-a)"
A_WORKSPACE_DIR="$(backend_workspace_dir mosaico-a)"
mkdir -p "${A_HOME}" "${A_WORKSPACE_DIR}"
# indexerRelay = the local relay: kind:0 profiles are read/written HERE, so a
# profile published to this relay is exactly what production would fetch from
# purplepag.es — the same code path in a closed world.
cat >"${A_CFG}" <<JSON
{
  "whitelistedPubkeys": ["${A_PK}"],
  "relays": ["${RELAY_WS}"],
  "indexerRelay": "${RELAY_WS}",
  "backendName": "mosaico-a",
  "userNsec": "${A_SK}",
  "mosaicoPrivateKey": "${A_SK}"
}
JSON
( cd "${A_WORKSPACE_DIR}" && mosaico mosaico-a channel init --force >/dev/null ) || die "channel init failed"
ok "mosaico-a pubkey ${A_PK}"
dim "  config=${A_CFG}"
dim "  workspace_dir=${A_WORKSPACE_DIR}  workspace_slug=${E2E_WORKSPACE}"

# ── 3. bootstrap: create the bare workspace group on the relay ──────────────────
# A plain session-start (no MOSAICO_CHANNEL) in the workspace dir drives
# the daemon opens the workspace root channel (kind:39000 d=slug)
# and adds mosaico-a's management key as an admin.
log "step 3: bootstrap session-start (creates room '${E2E_WORKSPACE}')"
BOOT_SID="warm-boot-$(date +%s)"
new_watch() { sleep 900 >/dev/null 2>&1 & LAST_WATCH=$!; WATCH_PIDS+=("$LAST_WATCH"); }
new_watch; WP_BOOT="$LAST_WATCH"
start_backend_daemon mosaico-a
(
  cd "${A_WORKSPACE_DIR}"
  printf '{"session_id":"%s","cwd":"%s","watch_pid":%s}' "${BOOT_SID}" "${A_WORKSPACE_DIR}" "${WP_BOOT}" \
    | MOSAICO_AGENT="claude" mosaico mosaico-a harness hook claude-code --type session-start
) || die "bootstrap session-start failed (see ${A_HOME}/daemon.log)"

if wait_for_soft "relay kind:39000 metadata for ${E2E_WORKSPACE}" 25 \
     "nak_req_contains '\"kind\":39000' -k 39000 -d '${E2E_WORKSPACE}' '${RELAY_WS}'"; then
  ok "room '${E2E_WORKSPACE}' exists on the relay (kind:39000)"
else
  die "room '${E2E_WORKSPACE}' never landed on the relay — cannot run warming checks"
fi

# Keep the daemon alive after the root channel exists: a long-lived channel
# reader holds the socket open between CLI calls while the demux loop receives
# the 39002 update and warms the peer's kind:0 profile.
mosaico mosaico-a channel read --live --channel "${E2E_WORKSPACE}" >/dev/null 2>&1 &
KEEPALIVE_PID=$!
sleep 1

# ── 4. publish the peer's kind:0 ONLY to the relay, then add it as a member ────
# The peer P is a pure relay identity: the backend has never hosted it and only
# learns its name by fetching its kind:0. Publishing to the local indexer relay is
# exactly what a human operator's client does against purplepag.es in production.
log "step 4: peer identity — publish kind:0, then add to '${E2E_WORKSPACE}'"
P_SK="$(nak key generate)"
P_PK="$(nak key public "${P_SK}")"
P_SHORT="${P_PK:0:8}"
dim "  peer pubkey ${P_PK}  (hex prefix ${P_SHORT})"

nak event -k 0 --sec "${P_SK}" -c "{\"name\":\"${PEER_NAME}\"}" "${RELAY_WS}" >/dev/null 2>&1 \
  || die "publishing the peer's kind:0 to the relay failed"
dim "  published kind:0 {name: ${PEER_NAME}}"

# mosaico-a is a group admin (whitelisted when the root channel opens), so its 9000
# put-user is accepted and the relay re-emits a signed 39002 that p-tags P.
nak event -k 9000 --sec "${A_SK}" -t "h=${E2E_WORKSPACE}" -t "p=${P_PK}" "${RELAY_WS}" >/dev/null 2>&1 \
  || die "9000 put-user for the peer failed"
dim "  mosaico-a admin sent 9000 put-user for the peer"

if wait_for_soft "relay kind:39002 to list the peer" 25 \
     "nak_req_contains '\"${P_PK}\"' -k 39002 -d '${E2E_WORKSPACE}' '${RELAY_WS}'"; then
  ok "peer is a relay member of '${E2E_WORKSPACE}' (kind:39002)"
else
  die "peer never landed as a relay member — cannot assert warming"
fi

# ── 5. CHECK 1: the daemon warmed the peer's kind:0 → who renders its NAME ─────
# `who --all-workspaces` reads the profile CACHE (it never fetches); the name only
# appears if the daemon proactively fetched the peer's kind:0 off the inbound
# 39002 and cached it. Poll to allow the async warm + relay echo to complete.
log "check 1: warming — peer resolves by name in the roster (not raw hex)"
if wait_for_soft "who to render @${PEER_NAME}" 25 \
     "mosaico mosaico-a who --all-workspaces 2>/dev/null | grep -q '@${PEER_NAME}'"; then
  check_pass "warming — peer's kind:0 fetched from the indexer; roster shows @${PEER_NAME}"
else
  WHO_OUT="$(mosaico mosaico-a who --all-workspaces 2>/dev/null || true)"
  echo "${WHO_OUT}" | sed 's/^/    /'
  if echo "${WHO_OUT}" | grep -q "@${P_SHORT}"; then
    check_fail "warming — peer still renders as hex @${P_SHORT}; kind:0 was not fetched/cached"
  else
    check_fail "warming — peer absent from roster (expected @${PEER_NAME})"
  fi
fi

# ── 6. CHECK 2: the backend's own management pubkey is excluded from the roster ─
log "check 2: backend exclusion — mgmt key '${A_PK:0:8}' not shown as a member"
WHO_OUT="$(mosaico mosaico-a who --all-workspaces 2>/dev/null || true)"
if echo "${WHO_OUT}" | grep -Eq "<member[^>]*@${A_PK:0:8}|@${A_PK:0:8}[^A-Za-z0-9]"; then
  echo "${WHO_OUT}" | sed 's/^/    /'
  check_fail "backend exclusion — mgmt key @${A_PK:0:8} leaked into the roster"
else
  check_pass "backend exclusion — mgmt key @${A_PK:0:8} absent from the roster"
fi

# ── summary ──────────────────────────────────────────────────────────────────
echo
for r in "${RESULTS[@]}"; do
  case "$r" in
    PASS*) printf '  %s%s%s\n' "$_c_green" "$r" "$_c_reset" ;;
    FAIL*) printf '  %s%s%s\n' "$_c_red"   "$r" "$_c_reset" ;;
  esac
done
printf 'totals: %sPASS=%d%s  %sFAIL=%d%s\n' "$_c_green" "$PASS_N" "$_c_reset" "$_c_red" "$FAIL_N" "$_c_reset"
if (( FAIL_N > 0 )); then
  die "${FAIL_N} hard check(s) failed"
fi
ok "PASS — proactive warming resolved a relay-only peer by name; mgmt key excluded"
