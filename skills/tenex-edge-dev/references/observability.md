# Observability

A live lab is only useful if it leaves enough evidence to debug. Use multiple
surfaces: agent UI, croissant logs, Nostr events, hook logs, and tenex-edge
daemon/relay logs.

## Evidence Surfaces

Minimum useful surfaces:

- portable PTY id plus attach/inject evidence for PTY sessions, or ACP smoke
  output plus the headless session id and delivery evidence for RPC sessions
- croissant relay log named in `lab.env`
- NIP-11 response from the relay
- `nak` event probes for the relevant kinds
- profile daemon and relay logs under `.container-state`
- hook-tail output when testing context injection

Do not rely on one source. For example, an agent UI can show injected context
while relay publication still fails. Croissant may show rejected events that do
not appear in tenex-edge logs. `nak` may show persisted events after the UI has
scrolled away.

## Inspecting Agent UI

For launch-mode runs, use the PTY id printed by `tenex-edge launch`:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge pty list
bash containers/tenex-edge/run --profile claude tenex-edge pty attach "${PTY_ID}"
```

To send a small prompt or hook-like injection from another terminal:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge pty inject "${PTY_ID}" \
  "Run tenex-edge my session and summarize the self header."
```

Keep prompts short and verifiable. Ask the agent to run one command or describe
one visible injection surface at a time. Direct-mode runs are foreground
auth/plugin checks; inspect them in the terminal where they are running.

## Inspecting ACP/App-Server Agents

Structured transports have no PTY UI. First capture the bundle smoke:

```bash
skills/tenex-edge-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

A pass includes the selected bundle/harness/transport, `initialize ok`, session
or thread creation, a completed model turn, and ACP cross-process resume. Then
capture the headless launch output:

```text
[tenex-edge acp] session: ...
[tenex-edge acp] headless agent launched; it responds to channel mentions (no PTY to attach)
```

While that container is alive, do not start a second container against the
same profile. Inspect the bind-mounted daemon/relay logs from the host and use
host `nak` probes. For delivery, correlate the mentioned kind:9 id with the ACP
session and the daemon's delivery/completion lines.

## Capturing Croissant

Relay log path comes from `lab.env`:

```bash
source "${LAB_ENV}"
tail -n 300 "${RELAY_LOG}"
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
bash containers/tenex-edge/run --profile claude shell -c \
  "curl -fsS -H 'Accept: application/nostr+json' '${RELAY_HTTP}'"
```

## Nostr Event Probes

Use the probe helper:

```bash
skills/tenex-edge-dev/scripts/probe-lab "${LAB_ENV}"
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
If hook-tail exits with a terminal/device error in a noninteractive run, use the
profile logs instead:

```bash
tail -n 200 .container-state/claude/tenex/edge/daemon.log
tail -n 200 .container-state/claude/tenex/edge/relay.log
```

Do not mark the lab failed on hook-tail alone when pty, daemon logs, relay logs,
and croissant traffic provide the evidence needed for the feature under test.

## Probe Directory Shape

`probe-lab` creates:

```text
probe-YYYYmmdd-HHMMSS/
  nip11.json
  relay.log
  kind-39000.jsonl
  kind-39001.jsonl
  kind-39002.jsonl
  kind-30315.jsonl
  kind-9.jsonl
```

Use file paths in the final report. If a file is empty, say that explicitly and
tie it to the likely cause. An empty event file can be meaningful evidence.

## Screenshots

For terminal evidence, attach output or a short terminal transcript is usually
better than a bitmap because it preserves text for search and review. Use a
screenshot only when the UI layout itself is under test or escape-sequence
rendering matters.

## Final Report Shape

Use this compact shape:

```text
Run:
- relay: ws://...
- profiles: claude-acp, codex-app-server
- mode: smoke + headless launch
- probe: /tmp/.../probe-...

Evidence:
- structured smoke: initialize/turn/resume ...
- ACP session <id>: delivery ...
- croissant: showed ...
- nak kind <kind>: showed ...
- hook-tail: showed ...

Result:
- pass/fail for the feature
- next failing command or next feature check
```

Avoid vague statements like "looks good" without a command, log, or captured
pane attached to the claim.
