//! Kind1 materializer — applies decoded `DomainEvent`s to the local store.
//!
//! Each method corresponds to one branch of the `handle_incoming` match; the
//! is_self guard and tail emission live in the top-level `materialize` dispatcher
//! in `fabric/mod.rs`, NOT inside these methods.

use crate::domain::{Mention, Presence, Profile, Status};
use crate::state::Store;
use nostr_sdk::Event;

pub struct Kind1Materializer;

impl Kind1Materializer {
    /// Apply a decoded `Profile` (kind:0) to the store.
    ///
    /// ACL logic: byte-identical to the Profile arm in `handle_incoming`.
    /// - Allowed → upsert_profile + remove_pending_agent.
    /// - Not blocked AND shares an owner with local `owners` → upsert_pending_agent.
    /// - Otherwise: no-op.
    pub fn materialize_profile(
        store: &Store,
        owners: &[String],
        pf: &Profile,
        now: u64,
    ) {
        let pk = &pf.agent.pubkey;
        if crate::acl::is_allowed(pk) {
            store.upsert_profile(pk, &pf.agent.slug, &pf.host, now).ok();
            store.remove_pending_agent(pk).ok();
        } else if !crate::acl::is_blocked(pk)
            && pf.owners.iter().any(|o| owners.contains(o))
        {
            store
                .upsert_pending_agent(pk, &pf.agent.slug, &pf.host, &pf.owners.join(","), now)
                .ok();
        }
    }

    /// Apply a decoded `Presence` (kind:1 presence variant) to the store.
    ///
    /// Byte-identical to the Presence arm in `handle_incoming`: expired events
    /// are silently ignored; otherwise upsert the peer session and (if slug is
    /// non-empty) upsert the profile too.
    pub fn materialize_presence(store: &Store, pr: &Presence, now: u64) {
        if pr.expires_at <= now {
            return;
        }
        store
            .upsert_peer_session(
                &pr.session_id,
                &pr.agent.pubkey,
                &pr.agent.slug,
                &pr.project,
                &pr.host,
                &pr.rel_cwd,
                now,
            )
            .ok();
        if !pr.agent.slug.is_empty() {
            store
                .upsert_profile(&pr.agent.pubkey, &pr.agent.slug, &pr.host, now)
                .ok();
        }
    }

    /// Apply a decoded `Status` to the store.
    ///
    /// Byte-identical to the Status arm in `handle_incoming`: expired statuses
    /// are silently ignored.
    pub fn materialize_status(store: &Store, st: &Status, now: u64) {
        if st.expires_at.map(|e| e <= now).unwrap_or(false) {
            return;
        }
        store
            .set_agent_status(&st.agent.pubkey, &st.project, &st.text, now)
            .ok();
    }

    /// Route an admitted mention into the local inbox.
    ///
    /// Delegates to `crate::runtime::route_mention_into` — that function remains
    /// the canonical implementation; this is a thin wrapper that lives inside
    /// the materializer so the dispatch path is uniform.
    ///
    /// Returns `true` if the mention was newly enqueued in at least one session
    /// inbox (i.e. the mention wake signal should fire).
    pub fn materialize_inbound_message(
        store: &Store,
        to_pubkey: &str,
        m: &Mention,
        event: &Event,
    ) -> bool {
        crate::runtime::route_mention_into(store, to_pubkey, m, event)
    }
}
