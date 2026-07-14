# Public Work Status

Read this reference when choosing or revisiting your public title, or when
another session's state affects how you coordinate with it.

## Name The Stable Outcome

Your title is the fabric's concise statement of the user-meaningful outcome you
currently own. It helps nearby participants understand your purpose, recognize
dependencies, and route relevant context.

- Set the title once the outcome is clear.
- Keep it to 15 words or fewer.
- Describe the outcome rather than the current tool, substep, or percentage
  complete.
- Keep it stable through implementation, tests, review, blockers, and ordinary
  progress.
- Update it when the owned outcome materially changes.

```bash
tenex-edge my session status "Publish and merge rewritten Tenex Edge skill"
```

Tenex Edge owns the live activity beneath that title and may occasionally ask
you to confirm whether a long-running title still fits.

## Interpret Session State

Presence summarizes what coordination can accomplish now:

- **working** — the session is actively taking a turn.
- **idle** — the session is online and directed attention can start a turn.
- **suspended** — the session remains online while new messages wait for manual
  resumption.
- **offline** — the session has ended or disconnected.

Use the surfaced state to choose between an immediate request, a durable
handoff, or another available participant. When you address a suspended
session, Tenex Edge surfaces the delivery consequence at send time.

When the decision calls for complete current fabric state rather than the
latest injected delta, inspect your session briefing:

```bash
tenex-edge my session
```

Sessions appear as the typed member rows inside the channels they have joined.

## End Or Re-home Only Yourself

Use self-lifecycle commands only for the current managed session:

```bash
tenex-edge my session end --self
tenex-edge my session kill --self
tenex-edge my session pty-wrap-me --self
```

`end` marks a session ended without killing its hosted process. `kill` stops
the hosted process. Use either only when the user asks to end the session or
the work is conclusively finished.

Use `pty-wrap-me` only when a session was started outside a daemon-owned PTY
and needs durable between-turn mention delivery. It kills the manually started
process and re-homes the same harness session, so use it only with explicit
need and after preserving anything that exists solely in terminal scrollback.
