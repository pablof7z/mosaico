//! Kind1 materializer — applies decoded `DomainEvent`s to the local store.
//!
//! Each method corresponds to one branch of the `handle_incoming` match; the
//! is_self guard and tail emission live in the top-level `materialize` dispatcher
//! in `fabric/mod.rs`, NOT inside these methods.

use crate::domain::{Mention, Profile, Status};
use crate::fabric::provider::FABRIC;
use crate::session::PeerStatusObservation;
use crate::state::Store;
use nostr_sdk::Event;

pub struct Kind1Materializer;

impl Kind1Materializer {
    /// Apply a decoded `Profile` (kind:0) to the store.
    ///
    /// NIP-29 admission happens at the relay/group layer. If a profile event is
    /// delivered by our scoped subscription, persist it for identity resolution.
    pub fn materialize_profile(store: &Store, pf: &Profile, now: u64) {
        let pk = &pf.agent.pubkey;
        store.upsert_profile(pk, &pf.agent.slug, &pf.host, now).ok();
    }

    /// Apply a decoded peer `Status` (kind:30315) to `peer_session_state`.
    ///
    /// `Status` is the single self-contained per-session signal, so one
    /// `record_peer_status` write mirrors the whole peer session: host/rel-cwd,
    /// title/activity/busy, and the liveness clock. The materializer ONLY ever
    /// touches `peer_session_state` — local sessions live in `session_state`,
    /// written exclusively by the daemon's transition methods.
    ///
    /// Liveness IS the freshness of the event: `emitted_at = seen_at` (the event
    /// `created_at`) drives `last_seen`, so re-fetching a persistent
    /// finished-session event does not resurrect it. `now` (local ingest) stamps
    /// `updated_at`/`first_seen`.
    ///
    /// Expired events are ignored for liveness: a status carrying a NIP-40
    /// expiration already past `now` describes a session that has aged off the
    /// fabric, so it must not refresh the peer mirror.
    ///
    /// The slug is NOT on the wire; it is resolved from the `profiles` table
    /// (populated by kind:0 events). Peer rows are NEVER seeded with a
    /// self-asserted slug — only kind:0 Profile events are authoritative.
    pub fn materialize_status(store: &Store, st: &Status, seen_at: u64, now: u64) {
        // Ignore expired events for liveness (NIP-40 expiration already past).
        if let Some(exp) = st.expires_at {
            if exp <= now {
                return;
            }
        }
        // Resolve slug from kind:0 profile (authoritative); fall back to empty.
        let slug = store
            .resolve_slug_for_pubkey(&st.agent.pubkey)
            .ok()
            .flatten()
            .unwrap_or_default();
        store
            .record_peer_status(&PeerStatusObservation {
                agent_pubkey: st.agent.pubkey.clone(),
                agent_slug: slug,
                project: st.project.clone(),
                native_session_id: st.session_id.as_str().to_string(),
                host: st.host.clone(),
                rel_cwd: st.rel_cwd.clone(),
                title: st.title.clone(),
                activity: st.activity.clone(),
                busy: st.busy,
                // Liveness clock: when the peer emitted this status.
                emitted_at: seen_at,
                // Local ingest time.
                observed_at: now,
            })
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
    /// Returns `(routed, thread_id)`: whether the mention was newly routed to a
    /// local inbox, and the canonical thread the message was filed under (None
    /// when the canonical dual-write failed).
    pub fn materialize_inbound_message(
        store: &Store,
        to_pubkey: &str,
        m: &Mention,
        event: &Event,
        provider_instance: &str,
        now: u64,
    ) -> (bool, Option<String>) {
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

        let thread_id = (|| -> anyhow::Result<String> {
            let project_id = store.ensure_project_origin(
                FABRIC,
                provider_instance,
                &m.project,
                &m.project,
                now,
            )?;
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
            store.add_message_recipient(
                &message_id,
                to_pubkey,
                m.target_session.as_ref().map(|s| s.as_str()),
            )?;
            Ok(thread_id)
        })()
        .ok(); // best-effort; never fail the legacy inbox path

        (routed, thread_id)
    }
}
