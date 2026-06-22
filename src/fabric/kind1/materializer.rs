//! Kind1 materializer — applies decoded `DomainEvent`s to the local store.
//!
//! Each method corresponds to one branch of the `handle_incoming` match; the
//! is_self guard and tail emission live in the top-level `materialize` dispatcher
//! in `fabric/mod.rs`, NOT inside these methods.

use crate::domain::{Profile, Status};
use crate::session::PeerStatusObservation;
use crate::state::Store;

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
        // Prefer the slug carried on the wire (session-signed status can't be
        // resolved via the author pubkey's kind:0); fall back to kind:0 profile
        // resolution for legacy emitters that didn't tag it.
        let slug = if !st.agent.slug.is_empty() {
            st.agent.slug.clone()
        } else {
            store
                .resolve_slug_for_pubkey(&st.agent.pubkey)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
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
}
