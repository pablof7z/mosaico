# Plan: Subgroup support in croissant (nip29.f7z.io)

## Context

We need "subgroups" on the `nip29.f7z.io` relay. Investigation of `/Users/pablofernandez/Work/croissant`
(the Go NIP-29 relay behind that domain) and its upstream dependency confirms **no notion of
parent/child groups exists anywhere** — neither in croissant nor in the read-only `fiatjaf.com/nostr/nip29`
package it embeds. Groups are a flat `map[groupID]*Group`.

Per decisions below, this is a **discovery/namespacing-only** feature: a child group carries a `parent`
pointer in its metadata so clients can render a hierarchy. The relay does **not** enforce any
membership/permission/read inheritance across the link, and does not cascade deletes. The only relay-side
rules are at *creation*: validating that the parent exists and that the creator is an admin of it.

### Decisions (locked with user)
- **Semantics:** discovery/namespacing only. Membership & permissions stay fully independent per group.
- **Creation:** only an **admin of the parent** may create a subgroup under it.
- **Depth:** arbitrary nesting. (See "Cycles" — with create-only parenting, cycles are impossible.)

## Design

There is **no NIP-29 standard for subgroups**, so this is a croissant convention:

- A child's **`kind:9007` create event** (`KindSimpleGroupCreateGroup`) carries a
  `["parent", "<parent-group-id>"]` tag. (Note: `9000` is `KindSimpleGroupPutUser`, not create — verified
  in `fiatjaf.com/nostr/kinds.go:334,338`. Create is `9007`, delete is `9008`.)
- The relay persists the parent link (it rides on the already-stored 9007 event) and re-emits it as a
  `["parent", "<parent-group-id>"]` tag on the child's `kind:39000` metadata event.
- Clients walk `parent` tags to build the tree.

> Reviewed via `codex exec` against the actual repo. Corrections folded in: create kind is 9007 (not
> 9000); cycles are **not** structurally impossible (delete+recreate can form one) so an explicit cycle
> check is required; plus several edge cases below.

**No upstream fork needed.** `nip29.Group` is a read-only module (`fiatjaf.com/nostr@v0.0.0-...`, no
`replace` directive in `go.mod`), so we do not touch it. The parent string lives on croissant's own
`Group` wrapper, and emission appends the tag to the event returned by `ToMetadataEvent()`.

**Persistence is free.** The `parent` tag is part of the signed 9007 create event, which the relay
already stores (`relay.StoreEvent` → `store.SaveEvent`, `relay.go:42`; derived state via `OnEventSaved`,
`relay.go:54`) and replays on boot via `loadGroupsFromDB`. We only need to parse it on the way in and
re-apply it on replay. (Caveat for tests: a test that calls `handleEventSaved` directly must also
`SaveEvent` the source 9007 create event, or restart/replay assertions won't see the parent.)

**Parent is set at creation only** (not editable via edit-metadata). This is the simplest correct design
and matches discovery-only intent. Re-parenting is an explicit optional extension (below).

## Changes

All paths under `/Users/pablofernandez/Work/croissant`.

### 1. `group.go` — store the parent link
- Add a field to the croissant `Group` struct (`group.go:24`), e.g. `Parent string` (guard reads/writes
  with the existing `mu` like other fields).
- Add a small helper near `getGroupIDFromEvent` (`group.go:167`). `Tags.Find` returns the first match, so
  multiple `parent` tags are first-wins (acceptable; optionally reject >1 in validation):
  ```go
  func getParentFromEvent(event nostr.Event) (string, bool) {
      if t := event.Tags.Find("parent"); t != nil && len(t) >= 2 && t[1] != "" {
          return t[1], true
      }
      return "", false
  }
  ```
- In `loadGroupsFromDB` (`group.go:59`), right after `group := s.NewGroup(groupId)` (line 67), read the
  parent from the create event `evt` and set `group.Parent` so it survives restarts.
- In `SyncGroupMetadataEvents` (`group.go:182`), the four metadata events are built as an array literal
  (lines 189–194); restructure so the kind:39000 event from `group.ToMetadataEvent()` gets
  `["parent", group.Parent]` appended when `group.Parent != ""`, **before** the
  `current.Tags.Eq(updated.Tags)` idempotency check and `Sign` (lines 195–208). Only the metadata kind
  gets the tag — leave admins/members/roles untouched.
- **Idempotency caveat (low risk for this feature):** `Tags.Eq` is strict length/order equality
  (`nostr/tags.go:128`) and `ToMetadataEvent` stamps `CreatedAt = LastMetadataUpdate`
  (`nip29/group.go:162`); `ReplaceEvent` only swaps if the stored event is older (`utils.go:104`). For
  **new** subgroups the first write is clean. We never mutate `parent` on existing groups, so no retrofit
  churn occurs here. (Pre-existing quirk, not introduced by us: croissant re-indexes `updated` after
  `ReplaceEvent` even when nothing was stored, `group.go:210` — out of scope.)

### 2. `process_event.go` — apply on live creation
- In the `KindSimpleGroupCreateGroup` branch (`process_event.go:16-52`), after `group = s.NewGroup(groupId)`
  (line 24), parse the parent via `getParentFromEvent(event)` and set `group.Parent`. (Validation has
  already passed by this point; this is just state population.)

### 3. `reject_event.go` — creation-time validation
- In the create branch (`reject_event.go:44-62`), before `return false, ""` (line 61), add: if the event
  has a `parent` tag, then
  - the parent group must exist (`State.Groups.Load(parentID)`) → else reject `"parent group doesn't exist"`;
  - the signer must be an **admin** of the parent: reuse the existing role pattern from this file
    (`roles := parentGroup.Members[event.PubKey]`, then
    `slices.ContainsFunc(roles, func(r *nip29.Role) bool { return r.Name == primaryRoleName })`) under
    `parentGroup.mu.RLock()` → else reject `"restricted: must be an admin of the parent group"`.
  - the link must not form a cycle: `wouldCreateCycle(groupId, parentId)` (see "Cycles & depth") → else
    reject `"restricted: would create a parent cycle"`.
  - Note: the relay root key already bypasses all checks at `reject_event.go:15`.

### Cycles & depth — explicit check REQUIRED
The original "cycles are structurally impossible" reasoning is **wrong** for this repo. Group IDs can be
recreated after deletion: delete removes the active group (`process_event.go:97`); the create duplicate
check only rejects *active* groups (`reject_event.go:41`) and then clears the deleted tombstone
(`process_event.go:24`). So this sequence forms a cycle: create `A` → create `B parent=A` → delete `A` →
recreate `A parent=B` (now `A→B→A`).

Therefore, in the kind:9007 create validation (`reject_event.go`), when a `parent` tag is present, **walk
the ancestor chain** upward from the proposed parent and reject if the new group's own id appears (or if a
chain step points at a non-existent group, which terminates the walk). Add a helper, e.g.
`func (s *GroupsState) wouldCreateCycle(newID, parentID string) bool` that follows `group.Parent` links
(bounded by a max-depth guard against malformed state). This honors arbitrary depth while preventing
cycles. Reject message: `"restricted: would create a parent cycle"`.

### Other edge cases (from review — handle or consciously accept)
- **Relay-root bypass:** the relay's own key skips all of `rejectEvent` (`reject_event.go:14`), so it can
  create subgroups bypassing parent-exists/admin/cycle checks. Acceptable (root is trusted); noted so it's
  not a surprise.
- **`LoadDeletedGroupState` loses `Parent`** (`group.go:129`): it rebuilds a deleted group from moderation
  events only and never parses the 9007 create event, so `Parent` would be empty. Low impact (group is
  deleted), but if the deleted-group view should show the parent, parse it from the create event there too.
- **Group ID validation:** `getGroupIDFromEvent` accepts any `h`/`d` value (`group.go:167`); we validate
  the *parent* id exists but don't tighten child-id format. Out of scope unless desired.

### Optional / out of scope (call out, don't silently drop)
- **Re-parenting** an existing group via edit-metadata: not included. If wanted later, reuse the same
  `wouldCreateCycle` walk plus handling in the `nip29.EditMetadata` case (`reject_event.go:167`) and a
  parent-tag parse in the metadata-apply path.
- **Cascade on parent delete:** not included (discovery-only ⇒ a dangling `parent` pointer is harmless;
  clients handle it).
- **`#parent` relay-side filtering:** the mmm store only indexes single-char tag names
  (`eventstore/mmm/helpers.go:209`), so `REQ` with `#parent` is not index-backed. Discovery works by
  fetching 39000s and walking tags client-side; fine for this design.
- **Croissant's own web UI** (`*.templ`) showing parent/children: optional; the data is in the 39000 tag.

## Verification

1. **Build:** `go build ./...` in `/Users/pablofernandez/Work/croissant`.
2. **Unit/integration:** add a test mirroring existing patterns (use the correct kinds: create = 9007
   `KindSimpleGroupCreateGroup`; the creator becomes admin via the synthetic 9000 put-user):
   - create parent group (9007);
   - create child (9007) with `["parent", parentID]` signed by the parent admin → expect accepted, and the
     child's emitted 39000 contains `["parent", parentID]`;
   - create child signed by a non-admin of the parent → expect rejected with the admin message;
   - create child with a non-existent parent → expect rejected;
   - create a grandchild under the child by the child's admin → accepted (arbitrary depth);
   - **cycle:** create `A`, create `B parent=A`, delete `A` (9008), recreate `A parent=B` → expect rejected
     by the cycle check.
3. **Restart/replay:** create a subgroup, restart the relay, confirm `loadGroupsFromDB` rebuilds
   `group.Parent` and re-emits the 39000 with the `parent` tag (query kind:39000 for the child's `d` tag).
4. **End-to-end against a local instance:** run the relay, use a nostr client/script to publish the 9007
   create with a `parent` tag, then `REQ` the child's kind:39000 and assert the `parent` tag is present.
   Confirm a flat (no-parent) group still emits a 39000 with **no** `parent` tag (no regression).
