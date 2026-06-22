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

A_PK="$(backend_pubkey edge-a)"
B_PK="$(backend_pubkey edge-b)"

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
nak req -k 39000 -d "${E2E_PROJECT}" "${RELAY_WS}" 2>/dev/null | grep -q '"kind":39000' \
  || die "parent group '${E2E_PROJECT}' not on relay — run ./e2e/run.sh first"
ok "parent '${E2E_PROJECT}' present; backends a=${A_PK:0:8} b=${B_PK:0:8}"

# ── 1. backend-a creates the subgroup ────────────────────────────────────────
log "1: backend-a create-group (research-lead@edge-a, testing-lead@edge-b)"
CG_OUT="$(edge edge-a project create-group \
  --parent "${E2E_PROJECT}" \
  --name "subgroup support" \
  --agent "research-lead@${A_PK}" \
  --agent "testing-lead@${B_PK}" 2>&1)" || { echo "${CG_OUT}" | sed 's/^/    /'; die "create-group failed"; }
echo "${CG_OUT}" | sed 's/^/    /'
CHILD_H="$(echo "${CG_OUT}" | grep -oE 'subgroup-support-[0-9a-f]{8}' | head -1)"
[[ -n "${CHILD_H}" ]] || die "could not parse child group id from create-group output"
ok "child group id: ${CHILD_H}"

# ── 2. child 39000 declares its parent ───────────────────────────────────────
log "2: child kind:39000 carries parent=${E2E_PROJECT}"
wait_for "child 39000 with a parent tag" 15 \
  "nak req -k 39000 -d '${CHILD_H}' '${RELAY_WS}' 2>/dev/null | grep -q '\"parent\"'"
nak req -k 39000 -d "${CHILD_H}" "${RELAY_WS}" 2>/dev/null | grep -q "\"${E2E_PROJECT}\"" \
  || die "child 39000 parent tag is not ${E2E_PROJECT}"
ok "child 39000 parent=${E2E_PROJECT}"

# ── 3. parent admins copied down to the child ────────────────────────────────
log "3: child kind:39001 admins include both backends"
ADMINS="$(nak req -k 39001 -d "${CHILD_H}" "${RELAY_WS}" 2>/dev/null)"
echo "${ADMINS}" | grep -q "${A_PK}" || die "edge-a is not a child admin"
echo "${ADMINS}" | grep -q "${B_PK}" || die "edge-b is not a child admin"
ok "child admins include edge-a and edge-b"

# ── 4. the single orchestration kind:9 ───────────────────────────────────────
log "4: orchestration kind:9 with te-op=subgroup.add-agents.v1"
wait_for "kind:9 add-agents orchestration event" 15 \
  "nak req -k 9 '${RELAY_WS}' 2>/dev/null | grep -q 'subgroup.add-agents.v1'"
ok "orchestration kind:9 present"

# ── 5. CROSS-BACKEND PROOF: backend-b provisioned testing-lead ───────────────
# backend-b shares no filesystem state with backend-a; the ONLY way it learns to
# mint testing-lead is the relayed kind:9 p-tagged to its backend identity.
log "5: backend-b minted testing-lead from the relayed kind:9 alone"
B_ROLE_JSON="$(backend_edge_home edge-b)/agents/testing-lead.json"
wait_for "backend-b to mint testing-lead identity" 25 "[[ -s '${B_ROLE_JSON}' ]]"
B_ROLE_PK="$(grep -oE '\"public_key\"[^0-9a-f]*[0-9a-f]{64}' "${B_ROLE_JSON}" | grep -oE '[0-9a-f]{64}')"
[[ -n "${B_ROLE_PK}" ]] || die "could not read testing-lead pubkey from ${B_ROLE_JSON}"
ok "backend-b minted testing-lead ${B_ROLE_PK:0:8}"

log "6: testing-lead is a MEMBER of the child group (added by backend-b)"
wait_for "child 39002 to include testing-lead" 25 \
  "nak req -k 39002 -d '${CHILD_H}' '${RELAY_WS}' 2>/dev/null | grep -q '${B_ROLE_PK}'"
ok "testing-lead is a child member"

# croissant is a PURE NIP-29 group relay: it stores group events only and drops
# non-group kind:0 metadata (in production the kind:0 goes to a separate
# indexerRelay like purplepag.es). The role's kind:0 publish is best-effort, so
# this is a soft check here — its absence reflects relay policy, not the feature.
log "7: testing-lead kind:0 profile (soft — croissant drops non-group kind:0)"
if wait_for_soft "testing-lead kind:0 profile" 6 \
     "nak req -k 0 -a '${B_ROLE_PK}' '${RELAY_WS}' 2>/dev/null | grep -q 'testing-lead'"; then
  ok "testing-lead kind:0 published"
else
  warn "no testing-lead kind:0 on croissant (expected: this relay stores group events only)"
fi

# ── 8. local fast-path: backend-a provisioned research-lead ──────────────────
log "8: backend-a provisioned research-lead locally"
A_ROLE_JSON="$(backend_edge_home edge-a)/agents/research-lead.json"
wait_for "backend-a to mint research-lead identity" 25 "[[ -s '${A_ROLE_JSON}' ]]"
A_ROLE_PK="$(grep -oE '\"public_key\"[^0-9a-f]*[0-9a-f]{64}' "${A_ROLE_JSON}" | grep -oE '[0-9a-f]{64}')"
wait_for "child 39002 to include research-lead" 25 \
  "nak req -k 39002 -d '${CHILD_H}' '${RELAY_WS}' 2>/dev/null | grep -q '${A_ROLE_PK}'"
ok "research-lead is a child member"

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
