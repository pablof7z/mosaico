#!/usr/bin/env bash
# e2e/run-subgroup.sh — NIP-29 subgroup task rooms (issue #3) across two backends.
#
# PREREQUISITE: ./e2e/run.sh has already run, so the relay + both backend daemons
# are live and the parent group 'e2e-demo' exists on the relay.
#
# This script has backend-a create a real NIP-29 SUBGROUP and publish ONE kind:9
# add-agents orchestration event naming a role on EACH backend. It then proves:
#   • the child group lands on the relay with parent=e2e-demo (relay re-emits it)
#   • the parent's trusted admins are copied down to the child
#   • backend-b — a separate install sharing NO filesystem state — receives the
#     orchestration event THROUGH THE RELAY and provisions its agent (mints the
#     durable role identity, publishes its kind:0, adds it as a child member).
# That last point is the cross-device auto-provisioning the feature exists for.

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

require_nak
command -v sqlite3 >/dev/null 2>&1 || die "sqlite3 not found on PATH — required to inspect backend state"

A_PK="$(backend_pubkey edge-a)"
B_PK="$(backend_pubkey edge-b)"

quote_for_sh() {
  local s="$1"
  printf "'%s'" "${s//\'/\'\\\'\'}"
}

role_harness_script() {
  local log_dir="$1"
  local bin qlog_dir
  bin="$(quote_for_sh "${TENEX_EDGE_BIN}")"
  qlog_dir="$(quote_for_sh "${log_dir}")"
  cat <<SCRIPT
cwd="\$(pwd)"
sid="\${TENEX_EDGE_PTY_SESSION:-e2e-role-\$\$}"
log_dir=${qlog_dir}
mkdir -p "\${log_dir}"
log="\${log_dir}/role-harness-\${TENEX_EDGE_AGENT:-agent}-\${sid}.log"
{
  printf 'role-start agent=%s sid=%s cwd=%s channel=%s\n' "\${TENEX_EDGE_AGENT:-}" "\${sid}" "\${cwd}" "\${TENEX_EDGE_CHANNEL:-}"
  printf 'role-env config=%s edge_home=%s tenex_dir=%s\n' "\${TENEX_CONFIG:-}" "\${TENEX_EDGE_HOME:-}" "\${TENEX_DIR:-}"
  printf '{"session_id":"%s","cwd":"%s","watch_pid":%s}\n' "\${sid}" "\${cwd}" "\$\$" | ${bin} harness hook claude-code --type session-start
  printf 'role-hook-rc=%s\n' "\$?"
} >>"\${log}" 2>&1
sleep 900
SCRIPT
}

seed_role() {
  local backend="$1" slug="$2" script
  script="$(role_harness_script "$(backend_edge_home "${backend}")/logs")"
  (
    cd "$(backend_project_dir "${backend}")"
    edge "${backend}" agent add "${slug}" -- \
      env \
      "TENEX_CONFIG=$(backend_config "${backend}")" \
      "TENEX_DIR=$(backend_tenex_dir "${backend}")" \
      "TENEX_EDGE_HOME=$(backend_edge_home "${backend}")" \
      "TENEX_EDGE_DEBUG=${TENEX_EDGE_DEBUG}" \
      /bin/sh -lc "${script}" >/dev/null
  )
}

# Keep backend-b's daemon ALIVE for the whole test. A real backend runs a
# persistent `tenex-edge daemon`; here edge-b's daemon was only auto-spawned by a
# CLI call and would idle-exit, cutting off the in-flight orchestration task
# mid-publish. A long-lived `tail` client holds the connection open so the daemon
# stays up to receive the kind:9 and finish provisioning. (edge-a stays alive via
# the synchronous create-group call below.)
KEEPALIVE_PID=""
cleanup_keepalive() { [[ -n "${KEEPALIVE_PID}" ]] && kill "${KEEPALIVE_PID}" 2>/dev/null || true; }
trap cleanup_keepalive EXIT
edge edge-b tail >/dev/null 2>&1 &
KEEPALIVE_PID=$!
sleep 1  # let the daemon come up and the tail attach

# ── preconditions (from run.sh) ──────────────────────────────────────────────
relay_up || die "relay not up — run ./e2e/run.sh first"
wait_for "parent kind:39000 metadata for ${E2E_PROJECT}" 20 \
  "nak_req_contains '\"kind\":39000' -k 39000 -d '${E2E_PROJECT}' '${RELAY_WS}'"
ok "parent '${E2E_PROJECT}' present; backends a=${A_PK:0:8} b=${B_PK:0:8}"

# Seed harmless local harness commands for the roles the orchestration event will
# spawn. Current orchestration proves completion through a real PTY-backed
# session_start, so a key-only agent is intentionally not enough.
log "0: seed local role harness commands"
seed_role edge-a research-lead || die "failed to seed research-lead command on backend-a"
seed_role edge-b testing-lead || die "failed to seed testing-lead command on backend-b"
ok "role harness commands seeded"

log "0b: backend-b session-start records its project root for spawned roles"
B_PROJ="$(backend_project_dir edge-b)"
B_BOOT_SID="subgroup-b-root-$$"
(
  cd "${B_PROJ}"
  printf '{"session_id":"%s","cwd":"%s","watch_pid":%s}\n' "${B_BOOT_SID}" "${B_PROJ}" "$$" \
    | TENEX_EDGE_AGENT="bootstrap-b" edge edge-b harness hook claude-code --type session-start >/dev/null
) || die "backend-b bootstrap session-start failed"
ok "backend-b project root recorded"

# ── 1. backend-a creates the subgroup ────────────────────────────────────────
log "1: backend-a channels create (research-lead@edge-a, testing-lead@edge-b)"
A_PROJ="$(backend_project_dir edge-a)"
CG_OUT="$(
  cd "${A_PROJ}"
  edge edge-a channels create \
    --name "subgroup support" \
    --about "subgroup support" \
    --agent "research-lead@${A_PK}" \
    --agent "testing-lead@${B_PK}" 2>&1
)" || { echo "${CG_OUT}" | sed 's/^/    /'; die "channels create failed"; }
echo "${CG_OUT}" | sed 's/^/    /'
wait_for "backend-a local channel cache to include subgroup support" 15 \
  "sqlite3 '$(backend_edge_home edge-a)/state.db' \"SELECT 1 FROM relay_channels WHERE parent='${E2E_PROJECT}' AND name='subgroup support' LIMIT 1;\" | grep -q 1"
CHILD_H="$(sqlite3 "$(backend_edge_home edge-a)/state.db" \
  "SELECT channel_h FROM relay_channels WHERE parent='${E2E_PROJECT}' AND name='subgroup support' ORDER BY updated_at DESC LIMIT 1;" \
  2>/dev/null || true)"
[[ -n "${CHILD_H}" ]] || die "could not resolve child group id from local channel cache"
ok "child group id: ${CHILD_H}"

# ── 2. child 39000 declares its parent ───────────────────────────────────────
log "2: child kind:39000 carries parent=${E2E_PROJECT}"
wait_for "child 39000 with a parent tag" 15 \
  "nak_req_contains '\"parent\"' -k 39000 -d '${CHILD_H}' '${RELAY_WS}'"
nak_req_contains "\"${E2E_PROJECT}\"" -k 39000 -d "${CHILD_H}" "${RELAY_WS}" \
  || die "child 39000 parent tag is not ${E2E_PROJECT}"
ok "child 39000 parent=${E2E_PROJECT}"

# ── 3. parent admins copied down to the child ────────────────────────────────
log "3: child kind:39001 admins include both backends"
wait_for "child 39001 to list edge-a as admin" 15 \
  "nak_req_contains '${A_PK}' -k 39001 -d '${CHILD_H}' '${RELAY_WS}'"
wait_for "child 39001 to list edge-b as admin" 15 \
  "nak_req_contains '${B_PK}' -k 39001 -d '${CHILD_H}' '${RELAY_WS}'"
ok "child admins include edge-a and edge-b"

# ── 4. the single orchestration kind:9 ───────────────────────────────────────
log "4: orchestration kind:9 with te-op=subgroup.add-agents.v1"
wait_for "kind:9 add-agents orchestration event" 15 \
  "nak_req_contains 'subgroup.add-agents.v1' -k 9 '${RELAY_WS}'"
ok "orchestration kind:9 present"

# ── 5. CROSS-BACKEND PROOF: backend-b provisioned testing-lead ───────────────
# backend-b shares no filesystem state with backend-a; the ONLY way it learns to
# mint testing-lead is the relayed kind:9 p-tagged to its backend identity.
log "5: backend-b minted testing-lead from the relayed kind:9 alone"
B_ROLE_JSON="$(backend_edge_home edge-b)/agents/testing-lead.json"
wait_for "backend-b to mint testing-lead identity" 25 "[[ -s '${B_ROLE_JSON}' ]]"
B_ROLE_BASE_PK="$(grep -oE '\"public_key\"[^0-9a-f]*[0-9a-f]{64}' "${B_ROLE_JSON}" | grep -oE '[0-9a-f]{64}')"
[[ -n "${B_ROLE_BASE_PK}" ]] || die "could not read testing-lead base pubkey from ${B_ROLE_JSON}"
wait_for "backend-b to start testing-lead session in child" 30 \
  "sqlite3 '$(backend_edge_home edge-b)/state.db' \"SELECT 1 FROM sessions WHERE agent_slug='testing-lead' AND channel_h='${CHILD_H}' AND alive=1 LIMIT 1;\" | grep -q 1"
B_ROLE_PK="$(sqlite3 "$(backend_edge_home edge-b)/state.db" \
  "SELECT agent_pubkey FROM sessions WHERE agent_slug='testing-lead' AND channel_h='${CHILD_H}' AND alive=1 ORDER BY created_at DESC LIMIT 1;" \
  2>/dev/null || true)"
[[ -n "${B_ROLE_PK}" ]] || die "could not read testing-lead session pubkey from backend-b state"
ok "backend-b minted testing-lead ${B_ROLE_BASE_PK:0:8} and started session pubkey ${B_ROLE_PK:0:8}"

log "6: testing-lead is a MEMBER of the child group (added by backend-b)"
wait_for "child 39002 to include testing-lead" 25 \
  "nak_req_contains '${B_ROLE_PK}' -k 39002 -d '${CHILD_H}' '${RELAY_WS}'"
ok "testing-lead is a child member"

# The NIP-29 relay is a PURE NIP-29 group relay: it stores group events only and drops
# non-group kind:0 metadata (in production the kind:0 goes to a separate
# indexerRelay like purplepag.es). The role's kind:0 publish is best-effort, so
# this is a soft check here — its absence reflects relay policy, not the feature.
log "7: testing-lead kind:0 profile (soft — NIP-29 relay drops non-group kind:0)"
if wait_for_soft "testing-lead kind:0 profile" 6 \
     "nak_req_contains 'testing-lead' -k 0 -a '${B_ROLE_PK}' '${RELAY_WS}'"; then
  ok "testing-lead kind:0 published"
else
  warn "no testing-lead kind:0 on NIP-29 relay (expected: this relay stores group events only)"
fi

# ── 8. local fast-path: backend-a provisioned research-lead ──────────────────
log "8: backend-a provisioned research-lead locally"
A_ROLE_JSON="$(backend_edge_home edge-a)/agents/research-lead.json"
wait_for "backend-a to mint research-lead identity" 25 "[[ -s '${A_ROLE_JSON}' ]]"
A_ROLE_BASE_PK="$(grep -oE '\"public_key\"[^0-9a-f]*[0-9a-f]{64}' "${A_ROLE_JSON}" | grep -oE '[0-9a-f]{64}')"
wait_for "backend-a to start research-lead session in child" 30 \
  "sqlite3 '$(backend_edge_home edge-a)/state.db' \"SELECT 1 FROM sessions WHERE agent_slug='research-lead' AND channel_h='${CHILD_H}' AND alive=1 LIMIT 1;\" | grep -q 1"
A_ROLE_PK="$(sqlite3 "$(backend_edge_home edge-a)/state.db" \
  "SELECT agent_pubkey FROM sessions WHERE agent_slug='research-lead' AND channel_h='${CHILD_H}' AND alive=1 ORDER BY created_at DESC LIMIT 1;" \
  2>/dev/null || true)"
wait_for "child 39002 to include research-lead" 25 \
  "nak_req_contains '${A_ROLE_PK}' -k 39002 -d '${CHILD_H}' '${RELAY_WS}'"
ok "research-lead is a child member (base ${A_ROLE_BASE_PK:0:8}, session ${A_ROLE_PK:0:8})"

# ── 9. channels list renders the hierarchy FROM LOCAL DAEMON STATE ────────────
log "9: backend-a 'channels list' shows the room under the project (from local state)"
wait_for "channels list to include ${CHILD_H} under ${E2E_PROJECT}" 15 \
  "edge edge-a channels list --project '${E2E_PROJECT}' 2>/dev/null | grep -q '${CHILD_H}'"
GL_OUT="$(edge edge-a channels list --project "${E2E_PROJECT}" 2>/dev/null)"
echo "${GL_OUT}" | sed 's/^/    /'
# The tree prints the project as the root with the child indented beneath it.
echo "${GL_OUT}" | grep -qE "^${E2E_PROJECT}$" || die "channels list missing project root"
echo "${GL_OUT}" | grep -qE "^  .*${CHILD_H}" || die "child not indented under the project"
ok "channels list renders the hierarchy from local daemon state"

cat <<SUMMARY

${_c_green}=== subgroup task rooms PASSED ===${_c_reset}
  parent        ${E2E_PROJECT}
  child         ${CHILD_H}   (39000 parent=${E2E_PROJECT})
  child admins  edge-a + edge-b (copied from parent)
  research-lead ${A_ROLE_PK:0:8}  provisioned by backend-a (local), child member
  testing-lead  ${B_ROLE_PK:0:8}  provisioned by backend-b (via relay), child member

backend-b minted + added its agent from the relayed kind:9 alone — cross-device
orchestration over a real NIP-29 relay, no shared filesystem state.
SUMMARY
