# Repository Guidance

This is the authoritative contributor guide for the repository — for agents and humans equally. Everything here is enforced, not suggested. `CLAUDE.md` defers to this file.

## No Backwards Compatibility

This repository does **not** preserve backwards compatibility. Removed surfaces
must be removed completely: no hidden aliases, parser aliases, legacy flags,
old subcommands, fallback JSON keys, duplicate MCP/tool names, stale e2e
commands, compatibility wrappers, or docs that continue teaching the old form.

If you find backwards-compatibility behavior, kill it immediately in the same
change. If a requested change would require adding a compatibility shim, do not
add it; change the caller, script, doc, or owning layer to the current contract
instead. Compatibility is drift, and drift is a bug.

## File Size

- Keep hand-authored source and documentation files under **300 lines of code** where practical (soft limit).
- Treat **500 lines of code** as a hard ceiling for hand-authored files.
- When a code file crosses the 500-LOC hard limit, refactor by splitting responsibilities along **domain boundaries** — cohesive ownership — not by moving arbitrary chunks to a sibling file.
- Inline tests that inflate a source file's LOC belong in a nested test module (e.g., a `tests` submodule or `tests.rs`), so the implementation stays under the soft target.
- Extracted module surfaces use **narrow visibility** (`pub(super)` or `pub(crate)`) rather than broad `pub` exposure. Only widen visibility when a consumer outside the module genuinely needs it.
- Integration-test helper files under `tests/` must live in a subdirectory and be loaded with explicit `#[path]` annotations, so they do not become standalone test crates.
- When a refactor runs `cargo fmt`, revert formatting churn on files unrelated to the change so the diff stays scoped.
- Generated, vendored, lockfile (`Cargo.lock`), binary, and benchmark-output artifacts are exempt from the LOC ceiling, but keep their producers small and documented.

## Planning discipline — GitHub queue, temporal files, no duplicate plans

This repository has exactly **one canonical tactical queue: GitHub Issues** (`gh issue list`). Every active violation, pending user decision, queued feature, post-v1 item, staged fix, and follow-up belongs in an open GitHub issue. Scattered notes, ad-hoc `TODO.md` / `NOTES.md` / `ROADMAP.md` / `PLAN-foo.md` files, parallel planning docs, and inline `// TODO:` annotations used as a substitute for tracking are **forbidden**.

Plans are not durable understanding and must not survive as reference documentation after they have been implemented, executed, or invalidated. When a plan completes, remove it or collapse it to the smallest live follow-up. Any lasting knowledge learned from the work belongs in durable documentation instead — `docs/product-spec/` for product doctrine, `docs/fabric-architecture*.md` and the design docs under `docs/` for architecture, and `docs/wiki/` for source-backed synthesis.

Rules — enforced strictly:

- **Do not create new top-level planning files.** No `PLAN.md`, `TODO.md`, `ROADMAP.md`, `NEXT.md`, or `STATUS.md` at the repo root or directly under `docs/`. New tactical detail belongs in a GitHub issue.
- **Do not duplicate state across files.** A violation or feature tracked in GitHub Issues is not also restated in a separate plan file. Single source of truth per fact.
- **GitHub search is the backlog view.** Use `gh issue list --state open` to see the queue; close or update the issue in the PR that touches it, and move lasting conclusions into durable docs.
- **A `// TODO:` comment is not a plan.** If it represents work to be done, it belongs in a GitHub issue. If it represents a known limitation or durable decision, it belongs in the architecture/design doc or wiki article that owns the concept.
- **Correct docs in place; do not layer superseding corrections.** When a doc is wrong, edit the authoritative document. Do not add an appendix or "revision" section whose job is to contradict the original while leaving the wrong text intact.
- **Never commit code reviews.** AI/codex review dumps and post-merge review notes must not be committed. Promote actionable findings into a GitHub issue or a durable doc, then discard the review.
- **Retire executed plans.** A plan that has been implemented is no longer a source of truth — close the issue or delete the temporal detail; preserve durable lessons in the doc that owns that concept.
- **When in doubt, fewer planning files.** The cost of a duplicate plan is divergence. If a new planning file feels necessary, justify why it cannot be a GitHub issue plus a durable doc.

> **Migration note.** The `Plans/` directory is empty and `M1.md` has been drained into `docs/daemon-design.md`, `docs/fabric-architecture.md`, and `docs/product-spec/`. Do not add new files under `Plans/` or new top-level plan docs; the canonical surfaces are GitHub Issues plus the durable docs.

## Daemon restart & deploy

A daemon restart (or binary swap) must **not** kill live agent/PTY sessions. The daemon and every detached PTY supervisor are the **same binary** (`mosaico`, re-invoked as `__pty-supervisor`), so killing by process name reaps the whole fleet.

- **Never kill the daemon by bare binary name.** A restart kills only the daemon process (`pkill -f 'mosaico daemon'`), never `pkill -x mosaico` / a process group / a cgroup. The daemon re-adopts the still-running supervisors on boot via `reconcile_sessions`.
- **systemd units must set `KillMode=process`** so stopping the unit signals only the main daemon PID, leaving the detached supervisors alive across the restart.
- `scripts/reset.sh` is a full **wipe** (deletes `state.db` and the sessions dir), so it deliberately reaps supervisors too — do not model a restart on it.
