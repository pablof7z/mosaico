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
- Name the result another participant or the user will be able to observe.
  “Inspecting files,” “running tests,” and “working on the branch” are activity,
  not outcomes.

```bash
mosaico my session status "Publish and merge rewritten Mosaico skill"
```

Mosaico owns the live activity beneath that title and may occasionally ask
you to confirm whether a long-running title still fits.

## Let The Branch Identify The Change

When Mosaico can safely resolve the session workspace as a Git checkout, it
derives the current branch and includes it in the injected `<self>` row. Do not
copy the branch name into the public title. The two fields answer different
questions:

- `title` says what meaningful outcome you own.
- `branch` says where that change is being developed.

Keep the title stable when you move between implementation, tests, review, and
merge on the same branch. Update it when the intended outcome changes. A
detached HEAD, an unavailable checkout, or an unsafe lookup simply omits the
branch; never invent one in the title to compensate.

## Interpret Session State

Presence summarizes what coordination can accomplish now:

- **working** — the session is actively taking a turn.
- **idle** — the session is online and directed attention can start a turn.
- **suspended** — the session remains online while new messages wait for manual
  resumption.
- **offline** — the session has ended or disconnected.

Use the surfaced state to choose between an immediate request, a durable
handoff, or another available participant. When you address a suspended
session, Mosaico surfaces the delivery consequence at send time.

When the decision calls for complete current fabric state rather than the
latest injected delta, inspect your session briefing:

```bash
mosaico my session
```

Sessions appear as the typed member rows inside the channels they have joined.

## End Or Re-home Only Yourself

Use self-lifecycle commands only for the current managed session:

```bash
mosaico my session end --self
mosaico my session kill --self
mosaico my session pty-wrap-me --self
```

`end` explicitly stops the runtime record without killing its hosted process;
harness exit hooks defer PTY-bound classification to the supervisor. `kill`
stops the hosted process. Use either only when the user asks to end the session
or the work is conclusively finished.

Use `pty-wrap-me` only when a session was started outside a daemon-owned PTY
and needs durable between-turn mention delivery. It kills the manually started
process and re-homes the same harness session, so use it only with explicit
need and after preserving anything that exists solely in terminal scrollback.
