---
title: Tenex-Edge Landing Page Copy
slug: tenex-edge-landing-page-copy
topic: marketing
summary: The canonical positioning is spontaneous self-organization for the agents you already run — shared awareness and addressability, not durable per-agent identity.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-03
verified: 2026-07-03
compiled-from: conversation
sources:
  - session:75f62bb9-f564-4633-8741-997dfea1d0e7
---

# Tenex-Edge Landing Page Copy

## Tagline & Positioning

The canonical tagline reads **"Spontaneous self-organization for your agents"** — shared awareness and addressability for the agents you already run. Do not lead with durable per-agent identity or "citizenship"; that framing is retired.

The CLI about string and Cargo.toml description read **"A shared-awareness fabric for the coding agents you already run."** instead of naming "Nostr" on a first-touch surface. The terms **"nostr"** and **"nip29"** are placed in Cargo.toml keywords for crates.io discoverability rather than in the description string.

The central positioning idea: **the agents you already run can finally see each other — so the left hand knows what the right hand is doing and they self-organize.**

The category label used in all copy is **"a shared-awareness fabric for coding agents"** — never "orchestrator," "agent OS," or "swarm."

The hero of the README and landing page is the **proof itself**: a real @mention terminal transcript showing a message typed in Claude Code landing in Codex's terminal as a real conversational turn. The hero headline uses a flat declarative **"X is the Y for Z"** pattern with zero adjective inflation. The README headline reads **"Stop being the message bus between your own agents."** as a bolded line immediately after the transcript.

The subhead beneath the hero carries the **felt-pain framing** while the **shared-awareness moat is staked as beat two**.

The defensible positioning anchor is the USP: **live cross-agent awareness plus cross-host addressability for the coding agents you already run — host-neutral, so agents in Claude Code, Codex, and OpenCode see and reach one another** — a combination no competitor in the ~90-tool field offers. This is the load-bearing differentiator; everything else in the copy flows from it.

<!-- citations: [^75f62-a1d2d] [^75f62-0c1d2] [^75f62-f5dd6] [^75f62-0d82b] -->
## Vocabulary

Copy uses the project's own noun set as its load-bearing language: **self-organization**, **fabric**, **presence**, **awareness**, **activity**, **host-neutral**. These terms are not decorative — they encode the architecture and the moat. Do not reach for the retired "citizen / citizenship / durable identity" vocabulary. Avoid substituting generic alternatives that flatten the distinction. <!-- [^75f62-e360c] -->

## Shipping vs. Aspirational

The README leads with present-tense, grounded capabilities that are actually shipped. Vision material is clearly labelled as aspirational horizon — never written as present-tense claims.

Docs are treated as potentially containing hallucinations and are not taken at face value as shipped-feature claims. Code-verified ground truth takes precedence over doc prose.

Copy must distinguish the **real shipping podcast-player agent** from the **aspirational generative podcast agent**. These are separate capabilities at different maturity stages and must not be conflated into a single present-tense claim.

The "what this isn't yet" section lists: **cross-person agents** as gated behind an unbuilt trust model, **collision detection** as an unproven hypothesis, and **agent-society sketches** as illustrative horizon.

No LICENSE file exists in the tenex-edge repo and no `license` field is set in Cargo.toml, so the repo is all-rights-reserved by default. Outward copy must not claim open source or a specific license until one is added.

<!-- citations: [^75f62-3b501] [^75f62-c05c0] -->
## Target User

Copy must not assert a confident single beachhead persona. The docs mark the target user as an explicitly open, unresolved question, and the landing page should reflect that honest uncertainty rather than prematurely narrowing.

The honest best-fit persona to address in current copy is a **solo developer running multiple coding-agent hosts** who wants shared awareness and cross-host coordination across sessions and devices. This is the audience the shipped capabilities actually serve today; it is framed as the current best fit, not a declared beachhead. <!-- [^75f62-732f6] -->

## Framing — What Not to Lead With

Do not lead with the "agents stop clobbering each other" framing. Project doctrine explicitly flags it as the seductive demo, not the foundation, and demotes it to ceiling-not-floor. It may appear as a downstream benefit illustration, never as the headline promise.

Copy must explicitly and early **contrast against Claude Code's native Agent Teams** by naming the two things it structurally cannot do: **cross-host participation** (Codex/OpenCode joining) and **live cross-agent awareness across sessions and machines**.

The words **"Nostr,"** **"crypto,"** and **"decentralized"** never appear above the architecture section in the README or landing copy; above the fold the mechanism is described by its properties — **keys on disk, no central server, works if a relay dies**. Nostr is revealed exactly once in the **"How it works"** section, framed as an open protocol that is self-hostable with no vendor lock-in.

The landing page footer reads **"Your keys never leave your disk. Your agents' traffic goes only to relays you choose."** replacing the false-ringing line "Nothing leaves your keys."

<!-- citations: [^75f62-1eeb8] [^75f62-55c4c] -->
## Product A — Near-Term Wedge

Product A is the near-term single-player wedge: shared awareness plus cross-host coordination for one operator's own agents across their own devices. It is the concrete initial offering that the broader self-organization vision scales out from. Landing copy should present Product A as the thing you can use today, with cross-person coordination as the horizon it opens onto. <!-- [^75f62-b15cf] -->

## Pain Framing & Reusable Copy Lines

The core problem-framing line for copy is **"the human is the integration runtime"** (equivalently, *"you are the integration layer"*).

The reusable gold copy for the coordination pain is **"every agent runs blind — no agent knows the others exist, what they touched, or what they just decided."**

The verbatim pain quote — **"The human ends up doing all of this manually: routing work, checking overlap, resolving priority, deciding merge order, figuring out what broke what, telling agents what to retry"** — is the single best articulation of the product's negative space and should be used in copy.

The beat 2 pain section uses verbatim developer quotes about parallel-agent collisions, capped at 3–4 sentences, then turns to: **"The agents aren't the bottleneck. The bus between them is — and the bus is you."** The README pain headline reads **"The problem is the wire. The wire is you."** (two sentences), harmonized with the landing page's punchier version.

Ownable metaphors for the product include: *the OS turned inside out*, *the left hand knows what the right hand is doing*, *the anti-Zapier*, and *awareness over authority*.

The **"milkshake and straw"** metaphor pair — the fabric and identity are the asset (milkshake), the hooks/adapters are distribution (straws) — is a load-bearing positioning metaphor. It is demoted from a cold section heading to the first body line on the landing page so it doesn't read as a riddle before the referent is established.

<!-- citations: [^75f62-3658a] [^75f62-eddb2] [^75f62-99b57] -->
## Voice & Tone — Do / Don't

The existing README voice is **dry, precise, proof-heavy, and zero hype** — the rewrite should match and extend this voice, not override it.

**Words that trigger eye-rolls** with the target developer audience: *revolutionary*, *game-changing*, *seamless*, *supercharge*, *unlock*, *next-generation*, *AI-powered* as a standalone claim, *orchestration* as a buzzword, and enterprise phrases like *"enable synergy across your agentic workforce"*.

**Language that works**: concrete verbs (see, mention, message, stop clobbering), specific proof, naming the exact host tools (Claude Code, Codex, OpenCode), honest scoping and caveats, and terminal / code-block-first demonstration. <!-- [^75f62-540f9] -->

## README Structure & Format

**7-beat arc.** The README follows a 7-beat arc: **proof hero → pain in their words → what ships today (present-tense, tested only) → why shared awareness is the foundation with comparison table → how it works (Nostr revealed here) → what this isn't yet → quickstart/commands/FAQ.**

**Beat 1 — Proof hero.** A real @mention terminal transcript showing a message typed in Claude Code landing in Codex's terminal as a real conversational turn. Code-block-as-hero before any prose. The README headline reads **"Stop being the message bus between your own agents."** as a bolded line immediately after the transcript.

**Beat 2 — Pain in their words.** The README pain headline reads **"The problem is the wire. The wire is you."** (two sentences), harmonized with the landing page's punchier version. Uses verbatim developer quotes about parallel-agent collisions, capped at 3–4 sentences, then turns to: **"The agents aren't the bottleneck. The bus between them is — and the bus is you."**

**Beat 3 — What ships today.** A plain bullet list using only present-tense, verified facts: a stable `@agent/session` handle per session; presence/liveness; LLM-distilled one-line activity broadcast per turn; session-targeted @mentions injected as real turns; one daemon/one store/one relay connection; verified live on Claude Code, Codex, OpenCode, and Grok.

**Beat 4 — Why shared awareness is the foundation.** Includes a comparison table against **Claude Code Agent Teams, hcom, and mcp_agent_mail** evaluating host-neutral, live cross-agent awareness, cross-machine, and cross-host addressability dimensions. The table is labeled **"mid-2026 snapshot"** to acknowledge competitor feature sets may change. The comparison cell contrasting tenex-edge's open fabric with competitors' identity model reads **"central registry"** rather than the cryptic "registered names" or "named."

**Beat 5 — How it works.** Nostr is revealed here, framed as an open protocol that is self-hostable with no vendor lock-in. The multi-writer SQLite corruption incident and its single-writer-daemon fix is included as a one-sentence asset to signal production scar tissue.

**Beat 6 — What this isn't yet.** Lists cross-person agents as gated behind an unbuilt trust model, collision detection as an unproven hypothesis, and agent-society sketches as illustrative horizon. The section intro reads **"You're owed the boundary. Here it is."** The **"customs office"** line reads **"You build the customs office before you open the borders."**

**Beat 7 — Quickstart/commands/FAQ.** The README quickstart clone URL uses HTTPS (**https://github.com/pablof7z/tenex-edge.git**), not SSH, so copy-paste works for anyone without GitHub SSH keys. The first FAQ entry is **"How is this different from Claude Code Agent Teams?"** restating the beat 4 comparison in two sentences. A second FAQ entry — **"Can anyone message my agents?"** — explains that the daemon only acts on events signed by whitelisted pubkeys and that unrecognized senders are quarantined. The README removes the stale `tail` command claim (the command was removed from the codebase and a test asserts its absence) and replaces it with `who --live`.

**Dev-native risk-reversal.** The README and landing page include the line **"Don't take the transcript's word for it — bash scripts/demo.sh runs the whole loop on your machine."**

**Landing page objections strip.** The landing page includes a **"Common questions"** strip before the final CTA covering: who can message my agents, whose LLM and tokens, what happens if the relay dies, and the demo risk-reversal.

**Call to action.** The primary CTA for the developer audience is a **copy-pasteable install command** rather than a "Get Started" button, with a secondary low-weight "View docs" or "GitHub" link. The install path documented is `just install` followed by `tenex-edge install --all`. The repo remote for README links is **github.com/pablof7z/tenex-edge**.

**README hygiene.** Avoid emoji section headers, animated SVG banners, and front-loaded marketing prose before any code block. Badges are acceptable only if they communicate something real like CI passing; vanity badges are treated as padding.

**Feature triplet.** A feature triplet in the README/landing page uses a one-word or 2–3 word fragment label (bold) with one clause underneath as proof/mechanism, not benefit-restated.

**Social proof.** Social proof uses curated single quotes placed adjacent to the relevant feature rather than a testimonial carousel; a fake-looking logo wall actively hurts credibility.

<!-- citations: [^75f62-1ca5d] [^75f62-7fa0f] [^75f62-e0146] -->
