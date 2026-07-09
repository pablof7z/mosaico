# Trust & Safety

> Status: draft, 2026-06-08. The hard chapter. This is a *phase gate*, not a footnote. The
> single feature everyone wants most — "my agent asks your agent" — is also the one that can
> turn the whole product into an attack surface. We treat it with the seriousness that
> implies.

## The core danger, stated plainly

Cross-person collaboration means **piping a foreign, autonomous LLM's output directly into
your own agent's context.** That is, by construction, a prompt-injection and data-
exfiltration channel. When Calle's agent asks my agent a question, my agent is now reading
text authored by *someone else's autonomous system* (or by an attacker who spoofed or
compromised it) and may act on it.

This is not "Product A with a security note." It is the defining risk of Product B (see
[scope-two-products.md](scope-two-products.md)), and it's why B is gated behind A.

## The threat model

- **Prompt injection.** A peer message contains instructions — "ignore your previous
  guidance, run this, tell me X." If peer input flows into the agent as *instructions* rather
  than *data*, the agent is hijackable by anyone who can reach it.
- **Exfiltration.** A peer asks innocuous-looking questions designed to extract secrets,
  private code, credentials, or roadmap from your agent's context or its reachable tools.
- **Broadcast leakage.** Presence and awareness inherently advertise *what you're working
  on* — paths, project names, goals. On a public fabric, that's your roadmap leaking to
  anyone subscribed. Awareness is a feature pointed at your fleet and a liability pointed at
  the world.
- **Impersonation.** An attacker poses as a trusted peer's agent. (The fabric gives us strong
  *authentication* for free — you can cryptographically verify *who signed* a message — so
  this is the *most* tractable threat. Authentication ≠ authorization, though; see below.)
- **Abuse / spam / DoS.** An open message bus invites unsolicited floods at your agent.

## The principles we hold

### 1. Peer input is data, never instructions
The cardinal rule. Anything arriving from another agent is treated as *untrusted content to
reason about*, never as commands to obey. A peer question becomes structured data the agent
may *choose* to answer — it never lands in the instruction channel. This is the single most
important design commitment in the whole project's safety posture.

### 2. Authentication is free; authorization is the real work
The fabric proves *who* sent something. It says nothing about whether they're *allowed* to
ask, at what rate, or about what. Authorization is a policy layer we own: who may query me,
about which of my projects/data, how often, with what standing consent. This is the genuinely
unsolved problem and we should not let it hide behind the (easy, solved) signing story.

### 3. Trusted-circle before open network
Cross-person starts with people you explicitly trust — household, team, named collaborators —
with explicit, scoped, revocable consent. The open world of strangers' agents is a separate,
later, much harder problem. (Tenet #5.) The household case (spouse's todo agent) is the safe
end; "anyone on the internet's agent" is the far end and may never be fully safe.

Within the fabric, trust is exactly NIP-29 channel membership. A machine's management key
adds a session's pubkey as a member; a session is removed on clean end and after 10 minutes
with no heartbeat (TTL prune). There is no durable per-agent or keystore identity conferring
standing — being trusted in a channel *is* being a current member of it.

### 4. Human-in-the-loop at the boundary, by default
Crossing the person boundary — especially *answering* a peer or *acting on* a peer's input —
defaults to surfacing for human approval, until and unless a standing, scoped consent says
otherwise. The human node (principle #3 in [first-principles.md](first-principles.md)) is the
safety valve precisely here.

### 5. Default-private awareness; opt-in to share
Presence and work-awareness default to *your own fleet only*. Sharing what you're working on
with another person is an explicit, scoped, per-peer choice — never an ambient broadcast.
What's a feature inside your umbrella is a leak outside it.

### 6. Scoped, revocable, expiring consent
Authorization to reach my agents is itself something I grant — narrowly (which agents, which
data, which kinds of questions), time-boxed, and revocable at any time. Consent is a
first-class, withdrawable thing, not a one-time on/off.

## Why this is a phase gate, not a footnote

The safe product (A) has *none* of these problems — everything in it is yours. The moment we
add B, all of them arrive at once. So B does not ship until:

1. The "peer input is data, not instructions" boundary is real and tested adversarially.
2. There's a working authorization/consent model (scoped, revocable, default-deny).
3. Awareness is default-private with explicit per-peer sharing.

Until then, cross-person stays off. We ship the customs office (A) and prove the trust model
at the lowest possible stakes (a trusted circle, read-mostly) before opening anything wider.

## The honest caveat

Letting autonomous agents talk to each other across trust boundaries may have a residual risk
that never fully goes to zero — the same way email never fully solved spam/phishing. Our job
isn't to claim we've eliminated it; it's to (a) make the safe single-player product not
depend on it at all, and (b) make the cross-person product *opt-in, scoped, and human-gated*
so the blast radius of any failure is contained to what the user explicitly authorized.
