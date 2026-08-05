# Observability

A useful lab leaves evidence at the provider, Mosaico, and relay boundaries.
Do not infer success from one surface.

## Safety boundary

An active launch container owns the daemon for its bind-mounted profile. Never
start a second container with that profile to inspect `sessions`, `channel`,
`debug explain`, `debug hook-tail`, or any other daemon-backed command. The
second daemon can replace the socket and evict the live session.

While the launch is alive, use:

- the terminal already attached to a PTY launch
- bind-mounted profile logs read directly from the host
- croissant logs and host `nak` requests
- files produced by `probe-lab`

Stop the launch container before same-profile CLI inspection.

## PTY and RPC evidence

For a PTY launch, capture the public session handle/id, the attached UI, and the
result of a narrow prompt. Do not open another container merely to reattach.

ACP/app-server has no PTY. First capture a bundle smoke:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

A pass includes the bundle, harness, transport, successful initialization,
session/thread creation, a completed turn, and resume. Then capture launch
output such as:

```text
[mosaico acp] session: ...
[mosaico acp] headless agent launched; it responds to channel mentions (no PTY to attach)
```

`headless` here describes the RPC presentation, not a launch flag. Correlate
the session id with daemon delivery/completion lines and relay events.

## Tagged delivery

Tag recipients structurally:

```bash
mosaico channel send --channel /<workspace>/<child> --tag <session-handle> \
  --message "Run mosaico my session."
```

Use a safe sender profile or already-running external backend. Literal
`@handle` message text is not a recipient tag. Use `--force` only to publish
mention-like text literally.

## Croissant log

```bash
source "${LAB_ENV}"
tail -n 300 "${RELAY_LOG}"
```

Look for connections, subscriptions, inbound/outbound event kinds, rejected
events, and rejection reasons. If an agent reports an action but croissant has
no traffic, check relay URL and container reachability.

## NIP-11 and container reachability

Host:

```bash
curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" | jq .
```

Before an agent is launched, the exact profile may check container reachability:

```bash
bash containers/mosaico/run --profile claude shell -c \
  "curl -fsS -H 'Accept: application/nostr+json' '${RELAY_HTTP}'"
```

Do not run that check against the profile after its launch container is alive.

## Nostr event probes

```bash
skills/mosaico-dev/scripts/probe-lab "${LAB_ENV}"
```

The helper captures NIP-11, the relay log, and selected kinds including channel
metadata/membership, profiles, and chat. Manual host probes are also safe:

```bash
nak req -k 39000 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 39001 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 39002 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 30315 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
nak req -k 9 "${RELAY_WS}" | jq -c '{kind,pubkey,tags,content}'
```

These may stream indefinitely; prefer the bounded helper.

## Profile logs

Read bind-mounted logs directly from the host:

```bash
tail -n 200 .container-state/claude-acp/mosaico/daemon.log
```

## Outgoing writes

There is no app-side log of them. NMP signs and publishes every group write, so
Mosaico never holds the bytes; what the daemon still owes is NMP's durable
publish queue, reported as `publish_queue` by `mosaico doctor` and rendered as
the `write.queue` check.

For an earlier failure, search the full file with timestamps. Keep logs until
the result has been reported.

## Hook forensics

After the launch container is stopped:

```bash
bash containers/mosaico/run --profile claude mosaico debug hook-tail
```

Use it for SessionStart, prompt/context injection, Stop/PostToolUse, installed
hook paths, and stale hook configuration. If its TUI cannot open, use daemon and
relay logs instead.

For Grok, also inspect `.container-state/grok/home/.grok/hooks/mosaico.json` and
confirm hook commands name `grok`, not `claude-code`. Correlate the hook process
with the launched PTY PID before treating it as session provenance.

Explain a published artifact with the current diagnostic path:

```bash
bash containers/mosaico/run --profile claude mosaico debug explain event:<id>
```

Again, run neither command concurrently with a same-profile launched agent.

## Probe directory

`probe-lab` writes a timestamped directory containing `nip11.json`, `relay.log`,
and event-kind JSONL files. Report its exact path. Empty event files are useful
only when paired with connection, config, and daemon evidence.

## Report shape

```text
Run:
- relay/run id
- profiles and bundles
- direct or launch mode
- probe directory

Evidence:
- provider auth and accepted command
- PTY handle or RPC session id
- croissant and Nostr event correlation
- daemon/hook evidence

Result:
- feature-specific pass/fail
- first failing command or next check
```
