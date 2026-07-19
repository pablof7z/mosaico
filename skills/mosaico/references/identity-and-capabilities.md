# Identity And Agent Capabilities

Read this reference when session identity, installed-agent discovery, backend
capabilities, or identity-bearing environment variables affect a decision.

## Use Public Identity Precisely

- A session's public identity is its Nostr pubkey. Its `npub` is the portable
  public form; its current `@handle` is a human-facing leased alias.
- Use an `npub`, hex pubkey, or current handle to select a session. PTY ids,
  harness resume tokens, process ids, and other runtime locators are not public
  identity and must not be taught or persisted as selectors.
- `MOSAICO_PUBKEY` anchors in-session CLI and local stdio MCP calls to the public
  session. A managed harness supplies it; do not replace it with a sibling
  session merely to make resolution succeed. Remote OAuth HTTP MCP calls derive
  their own first-class caller session and never require this environment anchor.
- `AGENT_NSEC` is the session signer. Treat it as a credential: never print,
  log, attach, commit, paste into chat, or forward to another participant. An
  agent normally has no reason to read or manipulate it.
- Authentication is not authorship. A human key used to authorize an external
  client does not automatically become the Mosaico session that publishes the
  client's actions.

## Interpret The Agent Inventory

- Discover capabilities on demand with `mosaico agents list`; the hook context
  deliberately does not embed the roster.
- `agent@backend` means the backend advertises an available capability. It is
  not a guarantee that launch will complete, a live session, channel member,
  lock, or proof that work is already assigned.
- Mosaico discovers valid Codex, Claude Code, and OpenCode native agent profiles
  from their global and workspace-local agent directories. These capabilities
  can appear without a duplicate Mosaico agent JSON.
- A workspace-local profile applies only in that workspace and takes precedence
  over the same harness's global profile there.
- Explicit Mosaico agents remain pinned to their configured harness bundle.
  Native profiles and generic detected harness agents acquire launch policy at
  realization time: interactive launch selects or creates PTY, while managed
  provisioning selects or creates the supported RPC transport. If one role is
  supplied by multiple harnesses without an explicit binding, launch remains
  ambiguous and must fail rather than silently choose one.
- Route by the advertised use criteria, workspace, and ownership. If dispatch
  reports missing, ambiguous, or incompatible activation, surface that exact
  failure; do not substitute a different role or backend invisibly.

## Respect Managed Delivery

- Managed PTY, ACP, and app-server sessions receive their assigned public key
  and signer at launch. Transport differences do not change their fabric
  authorship.
- Directed messages enter the recipient's inbox even while it is working, and
  pending delivery is replayed after a daemon restart. Do not duplicate a
  message merely because the recipient was busy or the daemon restarted.
- A daemon restart must target only the daemon process. Never kill every
  `mosaico` process, because detached PTY supervisors use the same binary.
