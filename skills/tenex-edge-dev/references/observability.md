# Observability

A live lab is only useful if it leaves enough evidence to debug. Use multiple
surfaces: agent UI, croissant logs, Nostr events, hook logs, and tenex-edge
daemon/relay logs.

## Evidence Surfaces

Minimum useful surfaces:

- host tmux pane for every agent session
- host tmux pane for the croissant relay
- NIP-11 response from the relay
- `nak` event probes for the relevant kinds
- profile daemon and relay logs under `.container-state`
- hook-tail output when testing context injection

Do not rely on one source. For example, an agent UI can show injected context
while relay publication still fails. Croissant may show rejected events that do
not appear in tenex-edge logs. `nak` may show persisted events after the UI has
scrolled away.

## Capturing Agent UI

Use host tmux session names printed by `launch-agent-tmux`:

```bash
tmux capture-pane -pt "${AGENT_TMUX}" -S -240 -e
```

The `-e` flag keeps escape sequences and screen state that can matter for TUIs.
If the capture is noisy, take a second plain capture:

```bash
tmux capture-pane -pt "${AGENT_TMUX}" -S -240
```

To send a small prompt:

```bash
tmux send-keys -t "${AGENT_TMUX}" "Run tenex-edge who and summarize the self header." C-m
```

Keep prompts short and verifiable. Ask the agent to run one command or describe
one visible injection surface at a time.

## Capturing Croissant

Relay tmux session comes from `lab.env`:

```bash
source "${LAB_ENV}"
tmux capture-pane -pt "${RELAY_TMUX}" -S -300 -e
```

Croissant is valuable because it logs traffic at the relay boundary. Look for:

- inbound event kinds
- subscriptions opened by agents
- outbound events
- rejected events
- rejection reasons
- relay URL/host binding mismatches

If the agent says something happened but croissant shows no traffic, check the
profile config relay URL and container network reachability.

## NIP-11

Check relay liveness:

```bash
curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" | jq .
```

NIP-11 proves the HTTP side is reachable from the host. It does not prove the
container can reach the WebSocket URL. For container reachability:

```bash
bash containers/tenex-edge/run --profile claude sh -lc 'curl -fsS -H "Accept: application/nostr+json" "$TENEX_EDGE_RELAY_HTTP"'
```

If that env var is not available in the runner, use the literal relay HTTP URL
from `lab.env`.

## Nostr Event Probes

Use the probe helper:

```bash
skills/tenex-edge-dev/scripts/probe-lab "${LAB_ENV}" "${AGENT_TMUX}"
```

It captures these kinds by default:

```text
39000
39001
39002
30315
9
```

Manual probes:

```bash
nak req -k 39000 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 39001 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 39002 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 30315 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 9 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
```

These commands can stream forever. Use the probe helper or stop them manually
after a few seconds.

## tenex-edge Logs

Profile-local logs:

```bash
tail -n 200 .container-state/claude/tenex/edge/daemon.log
tail -n 200 .container-state/claude/tenex/edge/relay.log
```

Use `tail` first. If a failure happened earlier, inspect the full log with
timestamps or copy it into a probe directory. Do not delete logs until the test
has been reported or the failure no longer matters.

## Hook Logs

Hook-tail:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge debug hook-tail
```

Use this when validating:

- SessionStart injection
- prompt/context injection
- Stop/PostToolUse/other hook behavior
- installed hook paths
- stale hook config failures

Hook proof should include both the hook-tail output and the agent UI evidence
when possible. The UI proves what the agent sees; hook-tail proves why it saw it.

## Probe Directory Shape

`probe-lab` creates:

```text
probe-YYYYmmdd-HHMMSS/
  nip11.json
  relay-pane.txt
  kind-39000.jsonl
  kind-39001.jsonl
  kind-39002.jsonl
  kind-30315.jsonl
  kind-9.jsonl
  tmux-<session>.txt
```

Use file paths in the final report. If a file is empty, say that explicitly and
tie it to the likely cause. An empty event file can be meaningful evidence.

## Screenshots

For terminal evidence, tmux pane capture is usually better than a bitmap because
it preserves text for search and review. Use a screenshot only when the UI
layout itself is under test or escape-sequence rendering matters. If using a
screenshot, still keep the text capture.

## Final Report Shape

Use this compact shape:

```text
Run:
- relay: ws://...
- profiles: claude, codex
- mode: launch
- probe: /tmp/.../probe-...

Evidence:
- tmux <session>: showed ...
- croissant: showed ...
- nak kind <kind>: showed ...
- hook-tail: showed ...

Result:
- pass/fail for the feature
- next failing command or next feature check
```

Avoid vague statements like "looks good" without a command, log, or captured
pane attached to the claim.
