# Authority and orientation

## Sources of truth

| Concern | Source |
|---|---|
| Contributor rules (compat, LOC, planning, daemon restart) | repo root `AGENTS.md` |
| Product doctrine | `docs/product-spec/` |
| Architecture | `docs/fabric-architecture.md`, `docs/fabric-architecture-overview.md`, design docs under `docs/` |
| Source-backed synthesis | `docs/wiki/` when present |
| Testing doctrine and commands | `skills/mosaico-dev/resources/testing/INDEX.md` |
| Container runner | `containers/mosaico/README.md` and `containers/mosaico/run` |
| Tactical backlog | open GitHub issues only — no parallel plan files |

Repo root `AGENTS.md` is enforced, not suggested. Do not restate its rules as a
second queue; correct durable docs in place when they drift.

## How to work

1. **Orient from authority.** Read the owning doc or module before inventing
   behavior. Prefer `docs/` and the repo root `AGENTS.md` over chat memory or
   stale plans.
2. **No backwards compatibility.** Remove dead surfaces completely in the same
   change. No aliases, legacy flags, fallback JSON keys, or dual names.
3. **One tactical queue.** Open or update a GitHub issue; do not create
   `TODO.md` / `PLAN-*.md` / scattered roadmaps. Retire executed plans.
4. **File size.** Soft 300 LOC, hard 500 LOC for hand-authored source. Split on
   domain boundaries; keep extracted visibility narrow.
5. **Daemon safety.** Never kill live PTY supervisors by bare binary name.
   Restart only the daemon process (`pkill -f 'mosaico daemon'`); see the repo
   root `AGENTS.md`.
6. **Secrets.** Never print provider credentials, Nostr secrets, `userNsec`,
   `mosaicoPrivateKey`, or agent private keys.
7. **Prove the right layer.** Unit/contract for pure rules; hermetic or local
   relay for process boundaries; live lab only for real-provider transport and
   auth. See `skills/mosaico-dev/resources/testing/INDEX.md`.

## Repo map

```text
AGENTS.md                 contributor contract (repo root)
docs/product-spec/        why and product shape
docs/fabric-architecture* how the fabric works
docs/harness-integration  provider/harness boundary
docs/daemon-*.md          daemon RPC and lifecycle
containers/mosaico/       isolated image + runner
skills/mosaico/           agent-facing fabric skill (shipped to users)
skills/mosaico-dev/       this skill (developer tooling)
e2e/                      black-box and behavior-contract surfaces
```

When unsure where a concept lives, search the repo and correct the owning doc —
do not invent a parallel note.
