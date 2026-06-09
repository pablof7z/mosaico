//! Kind1 materializer — applies decoded `DomainEvent`s to the local store.
//!
//! Each method corresponds to one branch of the `handle_incoming` match; the
//! is_self guard and tail emission live in the top-level `materialize` dispatcher
//! in `fabric/mod.rs`, NOT inside these methods.

use crate::domain::{Mention, Presence, Profile, Status};
use crate::fabric::provider::FABRIC;
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
    /// Expired events are silently ignored. The slug is NOT on the wire; it is
    /// resolved from the `profiles` table (populated by kind:0 events). Profile
    /// rows are NOT seeded from presence — only kind:0 is authoritative for slug.
    pub fn materialize_presence(store: &Store, pr: &Presence, now: u64) {
        if pr.expires_at <= now {
            return;
        }
        // Resolve slug from kind:0 profile (authoritative); fall back to empty.
        let slug = store
            .resolve_slug_for_pubkey(&pr.agent.pubkey)
            .ok()
            .flatten()
            .unwrap_or_default();
        store
            .upsert_peer_session(
                &pr.session_id,
                &pr.agent.pubkey,
                &slug,
                &pr.project,
                &pr.host,
                &pr.rel_cwd,
                now,
            )
            .ok();
        // Do NOT upsert_profile from presence — only kind:0 Profile events are
        // authoritative for the profiles table.
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

    /// Route an admitted mention into the local inbox AND dual-write a canonical
    /// inbound message row in the read-model tables.
    ///
    /// LEGACY PATH (authoritative): delegates to `crate::runtime::route_mention_into`
    /// — `inbox` + `seen_mentions` remain the authoritative reader tables (Phase 6).
    ///
    /// CANONICAL DUAL-WRITE (Phase 6 addition): after routing, also writes:
    ///   - `projects` / `project_origins` (idempotent)
    ///   - `threads` / `thread_origins` (idempotent; keyed by NIP-10 root `e` else event id)
    ///   - `messages` with `direction="inbound"`, `sync_state="received"` (idempotent on native_event_id)
    ///   - `message_recipients` (idempotent)
    ///
    /// Idempotency: `record_message` dedups on `native_event_id`, and
    /// `add_message_recipient` is INSERT OR IGNORE, so relay echo / refetch is safe.
    ///
    /// Returns `true` if the mention was newly enqueued in at least one legacy
    /// session inbox (i.e. the mention wake signal should fire). The canonical
    /// dual-write is unconditional — it does not depend on whether any sessions
    /// were alive when the event arrived.
    pub fn materialize_inbound_message(
        store: &Store,
        to_pubkey: &str,
        m: &Mention,
        event: &Event,
        provider_instance: &str,
        now: u64,
    ) -> bool {
        // ── Legacy path (AUTHORITATIVE — DO NOT CHANGE) ──────────────────────
        let routed = crate::runtime::route_mention_into(store, to_pubkey, m, event);

        // ── Canonical dual-write (Phase 6; readers stay on legacy until Phase 7) ─
        let eid_hex = event.id.to_hex();

        // NIP-10 root `e` tag → thread root; fall back to event id.
        let native_thread_key: String = event
            .tags
            .iter()
            .find_map(|t| {
                let s = t.as_slice();
                // ["e", <id>, <relay>, "root"] — standard NIP-10 marker
                if s.first().map(String::as_str) == Some("e")
                    && s.get(3).map(String::as_str) == Some("root")
                {
                    s.get(1).cloned()
                } else {
                    None
                }
            })
            .or_else(|| {
                // First bare `e` tag (no marker), per pre-NIP-10 usage.
                event.tags.iter().find_map(|t| {
                    let s = t.as_slice();
                    if s.first().map(String::as_str) == Some("e") {
                        s.get(1).cloned()
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| eid_hex.clone());

        let created_at = event.created_at.as_secs();

        (|| -> anyhow::Result<()> {
            let project_id =
                store.ensure_project_origin(FABRIC, provider_instance, &m.project, &m.project, now)?;
            let thread_id = store.ensure_thread_origin(
                &project_id,
                FABRIC,
                provider_instance,
                &native_thread_key,
                now,
            )?;
            let message_id = store.record_message(
                &thread_id,
                &m.from.pubkey,
                &m.body,
                created_at,
                "inbound",
                "received",
                Some(&eid_hex),
            )?;
            store.add_message_recipient(&message_id, to_pubkey, m.target_session.as_deref())?;
            Ok(())
        })()
        .ok(); // best-effort; never fail the legacy inbox path

        routed
    }
}
