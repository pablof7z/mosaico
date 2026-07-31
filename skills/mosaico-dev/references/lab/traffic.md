# Lab traffic: mentions, multi-agent, multi-human

## Tagged mention

The supported mention surface uses structured tags:

```bash
mosaico channel send --channel /<workspace>/<child> --tag <session-handle> \
  --message "Run mosaico my session."
```

Do this from a separate sender profile or external installed backend, not by
starting a second container against the live target profile. A literal
`@handle` in the message is rejected as ambiguous unless `--force` is used; it
does not create a recipient tag.

## Multi-agent

Smoke structured profiles sequentially. For live delivery, launch each target
in its own profile/container and use relay events or a separate sender profile
for cross-agent traffic. Keep prompts narrow; prove transport, event, and hook
behavior rather than task sophistication.

## Multi-human

After launching one target, send from each generated human by number or unique
name. The helper publishes that human's named kind:0 profile before its kind:9:

```bash
skills/mosaico-dev/scripts/send-human-kind9 "${LAB_ENV}" Pablo \
  <channel> <session-pubkey> "message from Pablo"
skills/mosaico-dev/scripts/send-human-kind9 "${LAB_ENV}" Alice \
  <channel> <session-pubkey> "message from Alice"
skills/mosaico-dev/scripts/send-human-kind9 "${LAB_ENV}" Bob \
  <channel> <session-pubkey> "message from Bob"
```

Prove identity at both boundaries: each relay event must have the selected
human's exact pubkey, and the agent-facing envelope must label the three
messages separately. Also verify one reply by event id so the return path uses
the originating event rather than a display-name guess.
