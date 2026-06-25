#!/usr/bin/env bash
# e2e/run.sh — boot a local NIP-29 relay + two isolated tenex-edge backends
# and prove they communicate THROUGH the relay with existing functionality.
#
# Pipeline:
#   0. teardown any previous run (idempotent)
#   1. build the NIP-29 relay if needed; start it on ws://127.0.0.1:$RELAY_PORT
#   2. mint keypairs for backend-a and backend-b
#   3. write each backend's isolated config.json + project registration
#   4. SMOKE TEST:
#        a. backend-a drives a session-start in the project dir
#           → daemon-a publishes kind:9007 create-group + 9000 put-user to relay
#        b. backend-b runs `project list`
#           → daemon-b fetches kind:39000 from the relay and sees the group
#      Backend B learning about A's group is only possible via the shared relay:
#      the two backends share NO filesystem state.
#
# Tunables: see e2e/lib.sh (RELAY_PORT, E2E_PROJECT, E2E_WORK, TENEX_EDGE_BIN…).

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

require_nak
[[ -x "${TENEX_EDGE_BIN}" ]] || die "tenex-edge binary not found at ${TENEX_EDGE_BIN} (run: cargo build)"

# ── 0. clean slate ───────────────────────────────────────────────────────────
log "step 0: tearing down any previous run"
E2E_KEEP_DATA=0 "${E2E_DIR}/teardown.sh" >/dev/null 2>&1 || true
mkdir -p "${E2E_WORK}" "${KEYS_DIR}"

# ── 1. relay ─────────────────────────────────────────────────────────────────
log "step 1: NIP-29 relay"
if [[ ! -x "${NIP29_RELAY_BIN}" ]]; then
  log "building NIP-29 relay (CGO; one-time, ~1m)"
  ( cd "${NIP29_RELAY_DIR}" && CGO_ENABLED=1 go build -o ./croissant ) || die "NIP-29 relay build failed"
fi
ok "NIP-29 relay binary: ${NIP29_RELAY_BIN}"

mkdir -p "${RELAY_DATA}"
# OWNER_PUBLIC_KEY must be a valid hex pubkey; we use backend-a's so it is also
# the relay's "owner" for the web UI. It does NOT grant event-publishing rights
# (the relay validates group writes by NIP-29 admin membership, not by owner).
OWNER_PK="$(backend_pubkey edge-a)"
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
A_SK="$(backend_seckey edge-a)"; A_PK="$(backend_pubkey edge-a)"
B_SK="$(backend_seckey edge-b)"; B_PK="$(backend_pubkey edge-b)"
ok  "edge-a pubkey ${A_PK}"
ok  "edge-b pubkey ${B_PK}"

# ── 3. per-backend config + project registration ─────────────────────────────
log "step 3: writing isolated configs"
write_backend() {
  local name="$1" sk="$2"
  local root edge tdir cfg proj
  root="$(backend_root "$name")"; edge="$(backend_edge_home "$name")"
  tdir="$(backend_tenex_dir "$name")"; cfg="$(backend_config "$name")"
  proj="$(backend_project_dir "$name")"
  mkdir -p "${edge}" "${tdir}" "${proj}"

  # Both backends' pubkeys are whitelisted on BOTH so each is a trusted admin on
  # every group. relays: only the local NIP-29 relay. tenexPrivateKey = this
  # backend's own identity (its pubkey becomes a group admin via open_project).
  cat >"${cfg}" <<JSON
{
  "whitelistedPubkeys": ["${A_PK}", "${B_PK}"],
  "relays": ["${RELAY_WS}"],
  "indexerRelay": "${RELAY_WS}",
  "backendName": "${name}",
  "userNsec": "${sk}",
  "tenexPrivateKey": "${sk}"
}
JSON

  # Register each isolated checkout in its backend-local project map so both
  # backends resolve the same slug independent of any git root.
  ( cd "${proj}" && edge "${name}" project init --force >/dev/null )
  dim "  ${name}: config=${cfg}"
  dim "          edge_home=${edge}  project_dir=${proj}"
}
write_backend edge-a "${A_SK}"
write_backend edge-b "${B_SK}"
ok "configs written (both whitelist both pubkeys; relays=[${RELAY_WS}])"

# Record daemon pids the first time each backend's daemon is auto-spawned, so
# teardown can stop them. We can't know the pid before the spawn, so we snapshot
# the newest __daemon after the first call below.
snapshot_daemon_pid() {
  local name="$1"
  local pid; pid="$(pgrep -n -f "${TENEX_EDGE_BIN} __daemon" || true)"
  [[ -n "$pid" ]] && echo "$pid" >"$(backend_pidfile "$name")"
}

# ── 4. smoke test ────────────────────────────────────────────────────────────
log "step 4: smoke test — two backends through one relay"

# 4a. backend-a: drive a claude-code session-start in the project dir. This is
#     the real production trigger for group creation: the session_start RPC
#     calls open_project, which publishes 9007 create-group then 9000 put-user.
log "4a: backend-a session-start (creates NIP-29 group '${E2E_PROJECT}')"
A_PROJ="$(backend_project_dir edge-a)"
A_SESSION="e2e-a-$(date +%s)"
session_start_payload() { printf '{"session_id":"%s","cwd":"%s"}' "$1" "$2"; }
(
  cd "${A_PROJ}"
  echo "$(session_start_payload "${A_SESSION}" "${A_PROJ}")" \
    | TENEX_EDGE_AGENT=claude edge edge-a hook --host claude-code --type session-start
) || die "backend-a session-start failed (see $(backend_edge_home edge-a)/daemon.log)"
snapshot_daemon_pid edge-a
ok "backend-a session-start completed"

# Confirm A's own daemon created the group (sanity: the publisher sees it).
log "4a-check: backend-a sees its own project"
wait_for_soft "backend-a project_list to include ${E2E_PROJECT}" 20 \
  "edge edge-a project list 2>/dev/null | grep -q '${E2E_PROJECT}'" || true
if edge edge-a project list 2>/dev/null | grep -q "${E2E_PROJECT}"; then
  ok "backend-a project list shows '${E2E_PROJECT}'"
else
  warn "backend-a does not yet list the project; dumping its view:"
  edge edge-a project list 2>&1 | sed 's/^/    /' || true
fi

# Independent confirmation the group really landed on the relay: query the relay
# directly for the relay-authored kind:39000 metadata event with d=<project>.
log "4a-relay: querying the relay directly for kind:39000 d=${E2E_PROJECT}"
if wait_for_soft "relay kind:39000 metadata for ${E2E_PROJECT}" 20 \
     "nak_req_contains '\"kind\":39000' -k 39000 -d '${E2E_PROJECT}' '${RELAY_WS}'"; then
  ok "relay holds kind:39000 metadata for '${E2E_PROJECT}' (group exists on the relay)"
else
  warn "no kind:39000 yet on the relay for '${E2E_PROJECT}':"
  nak req -k 39000 -d "${E2E_PROJECT}" "${RELAY_WS}" 2>&1 | sed 's/^/    /' || true
fi

# 4b. backend-b: a SEPARATE install with its own daemon + db. If it can list the
#     project, the only path the knowledge took was A → relay → B.
log "4b: backend-b project list (must observe A's group via the relay)"
B_OK=0
if wait_for_soft "backend-b project_list to include ${E2E_PROJECT}" 25 \
     "edge edge-b project list 2>/dev/null | grep -q '${E2E_PROJECT}'"; then
  B_OK=1
fi
snapshot_daemon_pid edge-b

echo
log "backend-b project list:"
edge edge-b project list 2>&1 | sed 's/^/    /' || true
echo

if [[ "${B_OK}" == "1" ]]; then
  ok "PASS — backend-b observed backend-a's group '${E2E_PROJECT}' through ${RELAY_WS}"
else
  die "backend-b never saw the group — backends are NOT communicating via the relay (logs: $(backend_edge_home edge-b)/daemon.log)"
fi

cat <<SUMMARY

${_c_green}=== smoke test PASSED ===${_c_reset}
  relay        ${RELAY_WS}   (pid $(cat "${RELAY_PIDFILE}"), log ${RELAY_LOG})
  backend-a    home=$(backend_edge_home edge-a)
  backend-b    home=$(backend_edge_home edge-b)
  project      ${E2E_PROJECT}

Inspect:
  edge() helper is in lib.sh; e.g.
    TENEX_CONFIG=$(backend_config edge-b) TENEX_EDGE_HOME=$(backend_edge_home edge-b) ${TENEX_EDGE_BIN} project list
  relay events:
    nak req -k 39000 ${RELAY_WS}
Tear down:
  ./e2e/teardown.sh            (wipes ${E2E_WORK})
  E2E_KEEP_DATA=1 ./e2e/teardown.sh   (keeps state for inspection)
SUMMARY
