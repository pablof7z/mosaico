# Session State

Every user-facing session surface uses one normalized state. Runtime facts stay
below this boundary and must not appear in agent-facing output.

| State | Contract |
|---|---|
| `working` | The session is online and mid-turn. |
| `idle` | The session is online, between turns, and Mosaico can automatically drive its attention. |
| `suspended` | The session is online and between turns, but has no live automatic-delivery path. Mentions remain queued until manual resume. |
| `offline` | The session is not live. |

The owning host is authoritative because it alone can classify runtime,
mid-turn activity, and automatic delivery together. Every local surface uses
the same lifecycle projection. The presence publisher signs that projection as
a kind:30315 lease for remote observers; it does not own a second state machine.

Fresh peer status is authoritative. A clean session end publishes `offline`
immediately, while NIP-40 expiration makes an unrefreshed state `offline` for
every viewer. Lease renewals refresh liveness without advancing the semantic
`updated_at` clock; only a title, activity, slug, or normalized-state change
produces a presence delta.

Local and remote observation intentionally differ during a partition: the
owning daemon can still know its runtime is live while remote viewers must mark
the expired lease offline. The signed lease carries `state-since`; rendered
`since` age is that semantic transition age, never the lease-renewal observation
time.

## Managed runtime lifecycle

The public state above is a projection, not the lifecycle store. A managed
session separately records its runtime incarnation, presentation, work state,
recovery authority, and per-channel fabric standing:

- `headed` means at least one client is attached to the PTY supervisor.
  Losing the last client only makes the still-running runtime `headless`; it
  does not unwrap or terminate the PTY child.
- A headless runtime becomes eviction-eligible only when it is idle and has no
  pending delivery. Ten minutes of that true inactivity conditionally stops
  the same runtime incarnation. Reattachment, a turn start, or accepted work
  cancels the old deadline.
- A stopped runtime normally retains its current channel memberships for one
  hour. A clean zero-status child exit while headed is treated as the user's
  intentional exit and removes current memberships immediately.
- Membership retention is fabric standing, while a session's recorded channel
  routes are recovery authority. Expiring standing does not erase the route,
  signer, or native conversation locator. An authorized exact p-tag re-admits
  the same pubkey after standing expires. When a native locator exists it also
  resumes that harness conversation; without one it starts a fresh harness
  conversation under the same durable session identity.
- Explicit forget or revoke is the destructive boundary. It removes recovery
  authority locally in one transaction and makes every recorded standing
  removal immediately due; unconfirmed relay removals remain durable retry
  work. Ordinary exit, eviction, and retention expiry do not revoke recovery.

Every deadline and runtime endpoint is fenced by the runtime/lifecycle
generation that created it. Supervisors persist exit reports before notifying
the daemon. A daemon restart replays those reports, reconciles a reserved
`stopping` transition, and resumes deadline processing without treating an
unavailable supervisor probe as proof that a session is headless.

The unavoidable distributed limit is abrupt host or network loss. A remote
viewer retains the last published state until expiration because, without a new
trusted signal, it cannot distinguish a partition from a silent host sooner.
