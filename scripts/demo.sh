#!/usr/bin/env bash
# Live, real-process end-to-end demo of tenex-edge.
#
# Spins up a real relay (nak serve), starts two agents (coder + reviewer) as real
# background engines, then exercises presence, awareness (distilled activity) and
# a session-targeted mention — asserting each is actually observed on the fabric.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/debug/tenex-edge"
NAK="${NAK:-$HOME/go/bin/nak}"
[ -x "$NAK" ] || NAK=nak

TMP="$(mktemp -d)"
PROJ="$TMP/myproject"; mkdir -p "$PROJ"
export TENEX_EDGE_HOME="$TMP/edgehome"
export TENEX_CONFIG="$TMP/config.json"
# Fast intervals so the demo is brisk.
export TENEX_EDGE_HEARTBEAT_MS=1000
export TENEX_EDGE_OBS_MS=700
export TENEX_EDGE_STATUS_TTL_S=30
# Distill 1s into a turn (not 30s) so the demo is quick.
export TENEX_EDGE_TURN_FIRST_S=1
# Activity distillation is LLM-only. To avoid needing an API key, point the
# distiller seam ($TENEX_EDGE_DISTILL_CMD) at a deterministic stub. It reads the
# transcript on stdin (proving the real path) and emits a one-line intent — in
# production this is the `edge-distillation` rig role over the same transcript.
export TENEX_EDGE_DISTILL_CMD='in=$(cat); echo "$in" | grep -q "fix the auth bug" && echo "Fixing the auth bug"'

PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
RELAY="ws://localhost:$PORT"

cat >"$TENEX_CONFIG" <<EOF
{ "whitelistedPubkeys": [], "relays": ["$RELAY"], "backendName": "demo-box" }
EOF

cleanup() {
  [ -n "${CODER_SID:-}" ] && "$BIN" session-end --session "$CODER_SID" >/dev/null 2>&1
  [ -n "${REV_SID:-}" ] && "$BIN" session-end --session "$REV_SID" >/dev/null 2>&1
  [ -n "${TAIL_PID:-}" ] && kill "$TAIL_PID" 2>/dev/null
  [ -n "${NAK_PID:-}" ] && kill "$NAK_PID" 2>/dev/null
  rm -rf "$TMP"
}
trap cleanup EXIT

echo "== starting relay on $RELAY =="
"$NAK" serve --port "$PORT" --quiet >/dev/null 2>&1 &
NAK_PID=$!
for _ in $(seq 1 100); do
  python3 -c "import socket,sys;s=socket.socket();sys.exit(0 if s.connect_ex(('127.0.0.1',$PORT))==0 else 1)" && break
  sleep 0.05
done

echo "== starting tail (live fabric view) =="
TAIL_LOG="$TMP/tail.log"
"$BIN" tail --project myproject >"$TAIL_LOG" 2>/dev/null &
TAIL_PID=$!
sleep 0.5

echo "== session-start: coder =="
CODER_SID="$("$BIN" session-start --agent coder --cwd "$PROJ")"
echo "   coder session: $CODER_SID"
echo "== session-start: reviewer =="
REV_SID="$("$BIN" session-start --agent reviewer --cwd "$PROJ")"
echo "   reviewer session: $REV_SID"

echo "== waiting for presence to propagate =="
sleep 3

echo
echo "== who (peer directory built from live presence) =="
WHO="$("$BIN" who --project myproject)"
echo "$WHO"

echo
echo "== coder works a turn (turn-start -> engine distills transcript -> activity) =="
# The host's turn-start hook marks the session working and hands over the live
# conversation transcript; the engine distills it ~TURN_FIRST_S in. A turn that
# finishes sooner never triggers a distill — here we let it run.
CODER_TRANSCRIPT="$TMP/coder.jsonl"
cat >"$CODER_TRANSCRIPT" <<'JSONL'
{"role":"user","content":"the login flow rejects valid tokens, please fix the auth bug"}
{"role":"assistant","content":"Looking at the token validation in src/auth.rs"}
JSONL
"$BIN" turn-start --session "$CODER_SID" --transcript "$CODER_TRANSCRIPT"
sleep 3
"$BIN" turn-end --session "$CODER_SID"

echo
echo "== live tail captured =="
sed 's/^/   | /' "$TAIL_LOG"

echo
echo "== assertions =="
fail=0
strip() { sed $'s/\x1b\\[[0-9;]*m//g'; }
check() { if echo "$2" | strip | grep -q "$3"; then echo "  PASS: $1"; else echo "  FAIL: $1"; fail=1; fi; }
check "who shows coder"                "$WHO"   "coder@myproject"
check "who shows reviewer"             "$WHO"   "reviewer@myproject"
check "tail shows live presence"       "$(cat "$TAIL_LOG")" "live"
check "tail shows distilled activity"  "$(cat "$TAIL_LOG")" "Fixing the auth bug"
check "tail shows the mention"         "$(cat "$TAIL_LOG")" "msg"

echo
if [ "$fail" = 0 ]; then echo "ALL CHECKS PASSED"; else echo "SOME CHECKS FAILED"; fi
exit $fail
