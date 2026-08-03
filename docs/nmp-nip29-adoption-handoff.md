# Handoff: adopt NMP's landed NIP-29 work

You are picking up the job of moving mosaico onto current NMP. Read `AGENTS.md`
and `CLAUDE.md` in this repo first — they govern how you work here, and nothing
below overrides them.

## Where things stand

`Cargo.toml` pins NMP at rev `9b3d2d25` (2026-07-29). NMP `master` is **153
commits ahead**. Mosaico depends on four crates from it:

| crate | commits since the pin |
|---|---|
| `nmp` | 37 |
| `nmp-nip29` | 8 |
| `nmp-grammar` | 4 |
| `nmp-network-policy` | 0 |

**NMP has a hard no-backwards-compatibility policy.** A replaced spelling is
deleted in the same change — no alias, no deprecation window, no forwarding
wrapper. Do not go looking for a compat shim; there isn't one, by design. Plan
for deletions, not warnings.

## The break you will hit immediately

`nmp_nip29::Group` **no longer exists.** It was deleted by NMP #1173, which
replaced the single-host NIP-29 door with a relay-scope object and composable
discovery predicates.

Mosaico has exactly one call site:

    src/nmp_host/write.rs:248 —  nmp_nip29::Group::new(relay, template.group)

That is the whole compile break as far as `nmp_nip29` goes, but do not assume
it is the whole *semantic* break — see "what actually changed" below.

### The shape that replaced it

The old `Group` bound a relay and a group id into one object that both read and
wrote. That is gone. The two directions are now separate, and neither carries a
relay inside it:

**Writing.** `nmp_nip29::operations::*` are free functions returning an unbound
`EventBuilder`:

    join_request(invite_code: Option<&str>) -> EventBuilder
    leave_request() -> EventBuilder
    add_user(pubkey: PublicKey, role: Option<&str>) -> EventBuilder
    remove_user(pubkey: PublicKey) -> EventBuilder
    edit_metadata(name: Option<&str>, about: Option<&str>) -> EventBuilder
    delete_event(event_id: nostr::EventId) -> EventBuilder
    create_group() / delete_group() / create_invite(code: &str) -> EventBuilder

The group context is applied as a separate, explicit step:

    nmp_nip29::contextualize(group_id: &str, builder: EventBuilder)
        -> Result<EventBuilder, GroupContextError>

`contextualize` **refuses** a builder that already carries an `h` tag or a
reserved timeline tag (`CallerSuppliedContext` / `CallerSuppliedTimeline`).
That refusal is deliberate: the group context is NMP's to apply, not the
caller's. If your porting instinct is to hand-write the `h` tag, that is
precisely what the API is stopping you from doing.

**Reading.**

    nmp_nip29::group_demand_at(host: &RelayUrl, group_id: &str, selection: Filter)
        -> Result<Demand, GroupContextError>

It refuses a `Filter` that already constrains the context tag
(`CallerSuppliedContextConstraint`), for the same reason.

Discovery predicates are composable and host-scoped:

    groups_where_at(host: &RelayUrl, predicate: Binding) -> Demand
    member_list_includes_at(...)
    admin_list_includes_at(...)
    GROUP_METADATA_KIND / GROUP_MEMBERS_KIND / GROUP_ADMINS_KIND

## What actually changed, beyond making it compile

Getting mosaico to build again is the small half. These are behaviour changes a
mechanical port will silently miss. Decide for each whether mosaico should
adopt it, and say so explicitly in your report even when the answer is "no
change needed".

**Multi-relay provenance is now correct (NMP #1221/#1222).** Until very
recently `Row.sources` meant *"relays that happened to deliver this event to
me"*, not *"relays in scope that hold it"* — reconciliation was seeded from a
relay-agnostic local store, so a relay that also held an event another relay
delivered first was never recorded as a source for it. If any mosaico logic
reasons about which relay a row came from, re-check it: the values it sees are
now different, and correct. Note the deliberate cost — an event two relays both
hold is now fetched from both, because the only thing substantiating "this
relay has it" is that relay serving it.

**A locally accepted write is visible immediately (NMP #1182/#1191).**
Provenance always distinguishes "in cache" from "came from these relays (zero
or more)". A cache-only row reports *yes in cache, relays: []* — a legitimate
state, not a missing value. Visibility under a host pin is decided by
ours-versus-foreign, not carried-versus-uncarried. If mosaico currently waits
for a relay acknowledgement before showing its own write, it no longer needs
to.

**One live query can carry independent branches (NMP #1108).** A query over
several hosts is one lifecycle with per-branch evidence rather than N separate
subscriptions. Given mosaico's use of `nmp::Subscription`, check whether
anything hand-rolled to fan out across relays is now the library's job.

**Per-branch evidence is positionally indexed.** `branches[i]` names the branch
that `evidence[i]` reports on. If you read evidence by position anywhere, that
correspondence is load-bearing — never reorder one without the other.

Other `nmp` symbols mosaico uses, all of which moved under those 37 commits and
should be re-verified rather than assumed: `Filter`, `Subscription`,
`WriteStatus`, `WriteIntent`, `RelayUrl`, `SourceStatus`, `Frame`,
`EventBuilder`, `AccessContext`, `fifo_channel`, `FACT_CHANNEL_CAPACITY`.

## How to do this

1. **Bump the pin first and let the compiler tell you the truth.** Update all
   four `rev = ` entries in `Cargo.toml` together to NMP master (`6ddc0de5` or
   later — check for newer; NMP master moves fast). Build. Collect the full
   error list before fixing anything; a partial fix against a moving surface
   wastes a cycle.
2. **Port the write path.** `write.rs:248` becomes an operations call plus
   `contextualize`. Let `GroupContextError` surface rather than unwrapping it —
   those refusals are real conditions, not defensive noise.
3. **Port the read path** onto `group_demand_at` and the discovery predicates.
4. **Then** work through the behaviour changes above deliberately.
5. **Run the real gates**, in the foreground, reading actual exit codes. This
   repo's `justfile` is the authority on what those are — do not invent a
   command. `${PIPESTATUS[0]}` is empty under zsh, so capture exit codes
   directly instead of piping into `tail` and reading the wrong status.

## Things that will cost you time if nobody says them

- **Use an isolated worktree with its own `CARGO_TARGET_DIR`.** Sharing one
  target dir across divergent branches has produced completely bogus build
  failures in this ecosystem.
- **Push early, even WIP.** Several pieces of finished work were found stranded
  on local disk in the NMP repo recently because their owner went away. A WIP
  commit costs nothing and is fully rewritable.
- **NMP master advances fast.** Merge it again before you finish, or a
  long-running port will be stale by the time it lands.
- Do not add a compatibility layer inside mosaico to smooth over the NMP
  change. If the new shape is awkward for mosaico's use, report that upstream
  as a design observation — wrapping it here just relocates the debt.

## Report

State: which call sites changed and why; which behaviour changes above mosaico
adopted, ignored, or deferred, with reasoning; anything in the new NMP surface
that made mosaico's job harder rather than easier (useful upstream signal); and
anything you could not verify.

If the port needs a design decision rather than a mechanical fix — particularly
around multi-relay reads or the write/context split — stop and say so rather
than guessing. An honest "this needs a call on X" is a good outcome.
