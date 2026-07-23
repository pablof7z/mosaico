#!/usr/bin/env bash
# e2e/run.sh — boot a local NIP-29 relay + two isolated mosaico backends
# and prove they communicate THROUGH the relay with existing functionality.
#
# Pipeline:
#   0. teardown any previous run (idempotent)
#   1. start the externally supplied NIP-29 relay on ws://127.0.0.1:$RELAY_PORT
#   2. mint keypairs for backend-a and backend-b
#   3. write each backend's isolated config.json + workspace registration
#   4. SMOKE TEST:
#        a. backend-a drives a session-start in the workspace dir
#           → daemon-a publishes kind:9007 create-group + 9000 put-user to relay
#        b. backend-b runs `channel list --all-workspaces`
#           → daemon-b fetches kind:39000 from the relay and sees the group
#      Backend B learning about A's group is only possible via the shared relay:
#      the two backends share NO filesystem state.
#
# Tunables: see e2e/lib.sh (RELAY_PORT, E2E_WORKSPACE, E2E_WORK, MOSAICO_BIN…).

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

require_nak
require_nip29_relay
[[ -x "${MOSAICO_BIN}" ]] || die "mosaico binary not found at ${MOSAICO_BIN} (run: cargo build)"

# ── 0. clean slate ───────────────────────────────────────────────────────────
log "step 0: tearing down any previous run"
E2E_KEEP_DATA=0 "${E2E_DIR}/teardown.sh" >/dev/null 2>&1 || true
mkdir -p "${E2E_WORK}" "${KEYS_DIR}"

# The harness launch scenario uses a deterministic Claude shim. Export its PATH
# and log before either isolated daemon starts so the detached supervisor inherits
# the same executable resolution environment.
export E2E_CLAUDE_ARGV_LOG="${E2E_WORK}/claude-argv.log"
export PATH="${E2E_DIR}/fixtures:${PATH}"

# ── 1. relay ─────────────────────────────────────────────────────────────────
log "step 1: NIP-29 relay"
ok "NIP-29 relay binary: ${NIP29_RELAY_BIN}"

mkdir -p "${RELAY_DATA}"
# OWNER_PUBLIC_KEY must be a valid hex pubkey; we use backend-a's so it is also
# the relay's "owner" for the web UI. It does NOT grant event-publishing rights
# (the relay validates group writes by NIP-29 admin membership, not by owner).
OWNER_PK="$(backend_pubkey mosaico-a)"
log "starting relay on ${RELAY_WS} (data: ${RELAY_DATA})"
# Launch the relay through nohup so follow-up scripts can reuse the live relay in
# ordinary shells. `env` execs the relay, so $! still tracks the long-lived
# listener process used by the stale-relay guard below.
nohup env PORT="${RELAY_PORT}" HOST="${RELAY_HOST}" DATAPATH="${RELAY_DATA}" \
    OWNER_PUBLIC_KEY="${OWNER_PK}" DOMAIN="" \
    "${NIP29_RELAY_BIN}" >"${RELAY_LOG}" 2>&1 &
RELAY_PID=$!
echo "${RELAY_PID}" >"${RELAY_PIDFILE}"

wait_for "relay NIP-11 to report supported_nips" 20 relay_up

# Guard against a stale relay: the process answering on the port MUST be the one
# we just started. An orphan relay (manual launch / crashed run) would bind
# the port first and serve OLD group state, making the test pass against the
# wrong relay. teardown reclaims the port, but verify here too.
LISTENER_PID="$(lsof -nP -tiTCP:"${RELAY_PORT}" -sTCP:LISTEN 2>/dev/null | head -1 || true)"
if [[ -n "${LISTENER_PID}" && "${LISTENER_PID}" != "${RELAY_PID}" ]]; then
  die "port ${RELAY_PORT} is held by pid ${LISTENER_PID}, not our relay (pid ${RELAY_PID}) — a stale relay is running. Run ./e2e/teardown.sh and retry."
fi
# Hard requirement for this rig: the relay MUST advertise NIP-29.
if curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" | grep -Eq '"supported_nips":[^]]*\b29\b'; then
  ok "relay up; NIP-11 advertises NIP-29"
else
  die "relay does not advertise NIP-29 in its NIP-11 doc — blocking (see ${RELAY_LOG})"
fi
RELAY_SK="$(grep -o '"relay_secret_key": *"[0-9a-f]*"' "${RELAY_DATA}/settings.json" | grep -o '[0-9a-f]\{64\}' || true)"
dim "  relay owner pubkey : ${OWNER_PK}"
[[ -n "${RELAY_SK}" ]] && dim "  relay signs 39000/39001/39002 with its own key (settings.json)"

# ── 2. keypairs ──────────────────────────────────────────────────────────────
log "step 2: backend keypairs"
A_SK="$(backend_seckey mosaico-a)"; A_PK="$(backend_pubkey mosaico-a)"
B_SK="$(backend_seckey mosaico-b)"; B_PK="$(backend_pubkey mosaico-b)"
ok  "mosaico-a pubkey ${A_PK}"
ok  "mosaico-b pubkey ${B_PK}"

# ── 3. per-backend config + workspace registration ─────────────────────────────
log "step 3: writing isolated configs"
write_backend() {
  local name="$1" sk="$2"
  local mosaico cfg workspace_dir
  mosaico="$(backend_mosaico_home "$name")"; cfg="$(backend_config "$name")"
  workspace_dir="$(backend_workspace_dir "$name")"
  mkdir -p "${mosaico}" "${workspace_dir}"

  # Both backends' pubkeys are whitelisted on BOTH so each is a trusted admin on
  # every group. relays: only the local NIP-29 relay. mosaicoPrivateKey = this
  # backend's own identity (its pubkey becomes a root-channel admin).
  cat >"${cfg}" <<JSON
{
  "whitelistedPubkeys": ["${A_PK}", "${B_PK}"],
  "relays": ["${RELAY_WS}"],
  "indexerRelay": "${RELAY_WS}",
  "backendName": "${name}",
  "userNsec": "${sk}",
  "mosaicoPrivateKey": "${sk}"
}
JSON

  # Register each isolated checkout in its backend-local workspace map so both
  # backends resolve the same slug independent of any git root.
  ( cd "${workspace_dir}" && mosaico "${name}" channel init --force >/dev/null )
  dim "  ${name}: config=${cfg}"
  dim "          mosaico_home=${mosaico}  workspace_dir=${workspace_dir}"
}
write_backend mosaico-a "${A_SK}"
write_backend mosaico-b "${B_SK}"
ok "configs written (both whitelist both pubkeys; relays=[${RELAY_WS}])"

# Agent files select a bundle and optional named profile. The bundle owns the
# underlying harness, transport, and operational args.
cat >"$(backend_mosaico_home mosaico-a)/harnesses.json" <<'JSON'
{
  "yolo-claude": {
    "harness": "claude-code",
    "transport": "pty",
    "args": ["--dangerously-skip-permissions"]
  }
}
JSON
mosaico mosaico-a agents add reviewer --harness yolo-claude --profile reviewer >/dev/null
ok "backend-a agent reviewer selects yolo-claude with profile reviewer"

# ── 4. smoke test ────────────────────────────────────────────────────────────
log "step 4: smoke test — two backends through one relay"

# 4a. backend-a: drive a claude-code session-start in the workspace dir. This is
#     the real production trigger for group creation: the session_start RPC
#     opens the workspace root channel, publishing create-group and membership events.
log "4a: backend-a session-start (creates NIP-29 group '${E2E_WORKSPACE}')"
A_WORKSPACE_DIR="$(backend_workspace_dir mosaico-a)"
A_SESSION="e2e-a-$(date +%s)"
session_start_payload() { printf '{"session_id":"%s","cwd":"%s"}' "$1" "$2"; }
start_backend_daemon mosaico-a
(
  cd "${A_WORKSPACE_DIR}"
  echo "$(session_start_payload "${A_SESSION}" "${A_WORKSPACE_DIR}")" \
    | MOSAICO_AGENT=claude mosaico mosaico-a harness hook claude-code --type session-start
) || die "backend-a session-start failed (see $(backend_mosaico_home mosaico-a)/daemon.log)"
ok "backend-a session-start completed"

# Confirm A's own daemon created the group (sanity: the publisher sees it).
log "4a-check: backend-a sees its own workspace"
wait_for_soft "backend-a channel_list --all-workspaces to include ${E2E_WORKSPACE}" 20 \
  "mosaico mosaico-a channel list --all-workspaces 2>/dev/null | grep -q '${E2E_WORKSPACE}'" || true
if mosaico mosaico-a channel list --all-workspaces 2>/dev/null | grep -q "${E2E_WORKSPACE}"; then
  ok "backend-a channel list --all-workspaces shows '${E2E_WORKSPACE}'"
else
  warn "backend-a does not yet list the workspace; dumping its view:"
  mosaico mosaico-a channel list --all-workspaces 2>&1 | sed 's/^/    /' || true
fi

# Independent confirmation the group really landed on the relay: query the relay
# directly for the relay-authored kind:39000 metadata event with d=<workspace>.
log "4a-relay: querying the relay directly for kind:39000 d=${E2E_WORKSPACE}"
if wait_for_soft "relay kind:39000 metadata for ${E2E_WORKSPACE}" 20 \
     "nak_req_contains '\"kind\":39000' -k 39000 -d '${E2E_WORKSPACE}' '${RELAY_WS}'"; then
  ok "relay holds kind:39000 metadata for '${E2E_WORKSPACE}' (group exists on the relay)"
else
  warn "no kind:39000 yet on the relay for '${E2E_WORKSPACE}':"
  nak req -k 39000 -d "${E2E_WORKSPACE}" "${RELAY_WS}" 2>&1 | sed 's/^/    /' || true
fi

# 4b. backend-b: a SEPARATE install with its own daemon + db. If it can list the
#     workspace, the only path the knowledge took was A → relay → B.
log "4b: backend-b channel list --all-workspaces (must observe A's group via the relay)"
B_OK=0
if wait_for_soft "backend-b channel_list --all-workspaces to include ${E2E_WORKSPACE}" 25 \
     "mosaico mosaico-b channel list --all-workspaces 2>/dev/null | grep -q '${E2E_WORKSPACE}'"; then
  B_OK=1
fi
record_backend_daemon_pid mosaico-b

echo
log "backend-b channel list --all-workspaces:"
mosaico mosaico-b channel list --all-workspaces 2>&1 | sed 's/^/    /' || true
echo

if [[ "${B_OK}" == "1" ]]; then
  ok "PASS — backend-b observed backend-a's group '${E2E_WORKSPACE}' through ${RELAY_WS}"
else
  die "backend-b never saw the group — backends are NOT communicating via the relay (logs: $(backend_mosaico_home mosaico-b)/daemon.log)"
fi

# 4c. Launch the configured role through the real daemon and PTY supervisor.
# The shim records exactly what the supervisor execs, proving the profile flag is
# supplied by code from (claude, pty), while permission args come from the bundle.
log "4c: harness-owned PTY launch applies the agent profile"
rm -f "${E2E_CLAUDE_ARGV_LOG}"
(
  cd "${A_WORKSPACE_DIR}"
  mosaico mosaico-a agents reviewer --workspace "${E2E_WORKSPACE}" >/dev/null
) || die "configured reviewer launch failed"
wait_for "Claude shim argv to be recorded" 10 "test -s '${E2E_CLAUDE_ARGV_LOG}'"
CLAUDE_ARGV="$(paste -sd ' ' "${E2E_CLAUDE_ARGV_LOG}")"
EXPECTED_CLAUDE_ARGV="--dangerously-skip-permissions --agent reviewer"
[[ "${CLAUDE_ARGV}" == "${EXPECTED_CLAUDE_ARGV}" ]] \
  || die "Claude argv mismatch: expected '${EXPECTED_CLAUDE_ARGV}', got '${CLAUDE_ARGV}'"
ok "PTY exec argv is claude ${CLAUDE_ARGV}"

cat <<SUMMARY

${_c_green}=== smoke test PASSED ===${_c_reset}
  relay        ${RELAY_WS}   (pid $(cat "${RELAY_PIDFILE}"), log ${RELAY_LOG})
  backend-a    home=$(backend_mosaico_home mosaico-a)
  backend-b    home=$(backend_mosaico_home mosaico-b)
  workspace      ${E2E_WORKSPACE}

Inspect:
  mosaico() helper is in lib.sh; e.g.
    MOSAICO_CONFIG=$(backend_config mosaico-b) MOSAICO_HOME=$(backend_mosaico_home mosaico-b) ${MOSAICO_BIN} channel list --all-workspaces
  relay events:
    nak req -k 39000 ${RELAY_WS}
Tear down:
  ./e2e/teardown.sh            (wipes ${E2E_WORK})
  E2E_KEEP_DATA=1 ./e2e/teardown.sh   (keeps state for inspection)
SUMMARY
