use crate::identity;
use anyhow::Result;
use nostr_sdk::prelude::Keys;
use std::collections::{HashMap, HashSet};

/// A reserved ordinal slot (issue #47). At most one LIVE session per
/// `(base agent pubkey, ordinal)`. Replaces the old binary
/// `(agent, project)` durable-vs-transient slot: instead of "first session gets
/// the durable key, everyone else gets a per-session transient key", each
/// concurrent live session takes the next free DURABLE ordinal identity
/// (`smith`, `smith1`, `smith2`, …), globally for that base agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct OrdinalSlot {
    base_pubkey: String,
    ordinal: u32,
}

/// In-memory reservation map: slot → owning session id. Tracks which ordinals
/// are live for each base agent so the allocator can pick the lowest free one
/// and two concurrent spawns can't both claim the same ordinal.
pub(super) type SignerReservations = HashMap<OrdinalSlot, String>;

pub(super) struct SignerRequest<'a> {
    pub session_id: &'a str,
    /// Durable base agent pubkey (the ordinal-0 identity).
    pub base_pubkey: &'a str,
    pub agent_slug: &'a str,
    /// The NIP-29 group id the session operates in (`route_scope`). This is used
    /// for membership admission and logging; ordinal identity itself is global.
    pub h: &'a str,
    /// Base agent keys — the HKDF derivation root for ordinals > 0.
    pub base_keys: &'a Keys,
    /// Honor this exact ordinal when free (resume of a known route, or a
    /// mention-driven spawn naming a specific `smithN`). `None` → allocate the
    /// lowest free global ordinal (birth).
    pub preferred_ordinal: Option<u32>,
    /// Pubkeys currently unavailable for reuse: live identities for the base
    /// agent plus pubkeys present in the target channel's active roster.
    pub occupied_pubkeys: Option<&'a HashSet<String>>,
    /// Pubkey this same session or exact preferred route is allowed to reuse.
    /// Generic births still treat roster membership as occupied; mention-driven
    /// exact routes must keep the pubkey that was p-tagged so replay can deliver.
    pub owned_pubkey: Option<&'a str>,
}

/// The identity selected for a session. Ordinal 0 signs with the base agent key
/// (`keys` is `None`, so callers fall back to the durable key via
/// `keys_for_session`); ordinal N>0 signs with a durable derived key and must be
/// admitted to the group as a member before use.
pub(super) struct SelectedSigner {
    pub ordinal: u32,
    pub pubkey: String,
    pub label: String,
    keys: Option<Keys>,
}

impl SelectedSigner {
    /// Project to the authoritative [`crate::identity::AgentInstance`] (issue #98).
    /// The base slug/pubkey are the agent's durable (ordinal-0) identity; this
    /// signer contributes the selected ordinal + pubkey. Engine + publishers
    /// derive label/pubkey/signing-key policy from the returned instance, never
    /// from the raw `label`/`keys` fields here.
    pub(super) fn instance(
        &self,
        base_slug: &str,
        base_pubkey: &str,
    ) -> crate::identity::AgentInstance {
        crate::identity::AgentInstance::from_parts(
            base_slug.to_string(),
            base_pubkey.to_string(),
            self.ordinal,
            self.pubkey.clone(),
        )
    }
    /// The pubkey that must be added as a NIP-29 member before use — only for
    /// ordinals > 0. Ordinal 0 is the base agent, admitted by the normal
    /// session-start membership flow.
    pub(super) fn member_pubkey_to_admit(&self) -> Option<&str> {
        if self.ordinal > 0 {
            Some(&self.pubkey)
        } else {
            None
        }
    }
}

fn slot(base_pubkey: &str, ordinal: u32) -> OrdinalSlot {
    OrdinalSlot {
        base_pubkey: base_pubkey.to_string(),
        ordinal,
    }
}

/// Construct the concrete signer for an ordinal: derive its durable keypair,
/// label, and pubkey. Ordinal 0 carries no engine keys (base-key fallback).
fn build(req: &SignerRequest<'_>, ordinal: u32) -> SelectedSigner {
    let keys = identity::derive_agent_ordinal_keys(req.base_keys, ordinal);
    let pubkey = keys.public_key().to_hex();
    let label = identity::agent_ordinal_label(req.agent_slug, ordinal);
    SelectedSigner {
        ordinal,
        pubkey,
        label,
        keys: if ordinal == 0 { None } else { Some(keys) },
    }
}

/// Commit a chosen ordinal: record its engine keys (or clear them for ordinal 0)
/// and return the signer. The reservation must already be inserted.
fn finish(
    session_keys: &mut HashMap<String, Keys>,
    req: &SignerRequest<'_>,
    ordinal: u32,
) -> SelectedSigner {
    let signer = build(req, ordinal);
    match &signer.keys {
        Some(k) => {
            session_keys.insert(req.session_id.to_string(), k.clone());
        }
        None => {
            session_keys.remove(req.session_id);
        }
    }
    signer
}

fn is_taken(r: &SignerReservations, req: &SignerRequest<'_>, ordinal: u32) -> bool {
    if r.get(&slot(req.base_pubkey, ordinal))
        .map(String::as_str)
        .is_some_and(|owner| owner != req.session_id)
    {
        return true;
    }
    let signer = build(req, ordinal);
    req.occupied_pubkeys
        .is_some_and(|pks| pks.contains(&signer.pubkey))
        && req.owned_pubkey != Some(signer.pubkey.as_str())
}

/// Lowest ordinal not currently reserved by a live session or otherwise visible
/// as occupied.
fn lowest_free(r: &SignerReservations, req: &SignerRequest<'_>) -> u32 {
    let mut n = 0u32;
    while is_taken(r, req, n) {
        n += 1;
    }
    n
}

/// Select and reserve a global ordinal identity for a session.
///
/// - Reassert: if this session already owns a slot, keep its ordinal.
/// - `preferred_ordinal` (resume/mention): honor it when free.
/// - Otherwise allocate the lowest free global ordinal (birth).
///
/// Race-safe under the single-writer daemon: the caller holds the reservations +
/// session_keys mutexes across this call, so two concurrent spawns serialize and
/// cannot pick the same ordinal.
pub(super) fn select_and_reserve(
    reservations: &mut SignerReservations,
    session_keys: &mut HashMap<String, Keys>,
    req: SignerRequest<'_>,
) -> Result<SelectedSigner> {
    // Reassert: this session already holds an ordinal.
    if let Some(existing) = reservations.iter().find_map(|(s, owner)| {
        (s.base_pubkey == req.base_pubkey && owner.as_str() == req.session_id).then_some(s.ordinal)
    }) {
        tracing::debug!(
            session = %req.session_id,
            agent = %req.agent_slug,
            h = %req.h,
            ordinal = existing,
            "ordinal reasserted"
        );
        return Ok(finish(session_keys, &req, existing));
    }

    let ordinal = match req.preferred_ordinal {
        Some(n) if !is_taken(reservations, &req, n) => n,
        // Preferred ordinal is occupied by a different live session. Channel
        // switch rejects this upstream; at birth it should not happen, but fall
        // back to lowest-free to keep the session live rather than failing.
        Some(preferred) => {
            tracing::warn!(
                session = %req.session_id,
                agent = %req.agent_slug,
                h = %req.h,
                preferred,
                "preferred ordinal occupied by another session; falling back to lowest-free"
            );
            lowest_free(reservations, &req)
        }
        None => lowest_free(reservations, &req),
    };
    reservations.insert(slot(req.base_pubkey, ordinal), req.session_id.to_string());
    let signer = finish(session_keys, &req, ordinal);
    tracing::info!(
        session = %req.session_id,
        agent = %req.agent_slug,
        h = %req.h,
        ordinal,
        label = %signer.label,
        "ordinal slot allocated"
    );
    Ok(signer)
}

/// Release a session's reservation + engine keys (session end / failure / GC).
/// Scans by session id, so it does not need the room or ordinal.
pub(super) fn release(
    reservations: &mut SignerReservations,
    session_keys: &mut HashMap<String, Keys>,
    session_id: &str,
) -> Option<Keys> {
    let freed: Vec<u32> = reservations
        .iter()
        .filter(|(_, owner)| owner.as_str() == session_id)
        .map(|(s, _)| s.ordinal)
        .collect();
    reservations.retain(|_, owner| owner != session_id);
    for ordinal in freed {
        tracing::info!(session = %session_id, ordinal, "ordinal slot released");
    }
    session_keys.remove(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_keys() -> Keys {
        Keys::new(nostr_sdk::prelude::SecretKey::from_slice(&[0x22; 32]).unwrap())
    }

    fn request<'a>(
        session_id: &'a str,
        h: &'a str,
        base_pubkey: &'a str,
        base_keys: &'a Keys,
        preferred_ordinal: Option<u32>,
    ) -> SignerRequest<'a> {
        SignerRequest {
            session_id,
            base_pubkey,
            agent_slug: "smith",
            h,
            base_keys,
            preferred_ordinal,
            occupied_pubkeys: None,
            owned_pubkey: None,
        }
    }

    #[test]
    fn first_session_in_room_is_ordinal_zero_base_key() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let s = select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        assert_eq!(s.ordinal, 0);
        assert_eq!(s.label, "smith");
        assert_eq!(s.pubkey, bp); // ordinal 0 IS the base pubkey
        assert!(s.member_pubkey_to_admit().is_none());
        assert!(sk.is_empty());
        // Projected instance: ordinal 0 signs with the base keys (no derivation).
        let inst = s.instance("smith", &bp);
        assert_eq!(inst.display_slug(), "smith");
        assert_eq!(inst.pubkey, bp);
        assert_eq!(
            inst.signing_keys(&bk).secret_key().to_secret_hex(),
            bk.secret_key().to_secret_hex()
        );
    }

    #[test]
    fn second_same_agent_same_room_is_ordinal_one_durable() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        let s2 = select_and_reserve(&mut r, &mut sk, request("s2", "#a", &bp, &bk, None)).unwrap();
        assert_eq!(s2.ordinal, 1);
        assert_eq!(s2.label, "smith1");
        assert_ne!(s2.pubkey, bp);
        assert_eq!(s2.member_pubkey_to_admit(), Some(s2.pubkey.as_str()));
        // Durable: the same ordinal-1 key is reproducible (room-independent).
        assert_eq!(
            s2.pubkey,
            identity::derive_agent_ordinal_keys(&bk, 1)
                .public_key()
                .to_hex()
        );
        // Projected instance: ordinal 1 signs with a DERIVED key whose pubkey is
        // the selected pubkey — never collapsing back onto the base.
        let inst = s2.instance("smith", &bp);
        assert_eq!(inst.display_slug(), "smith1");
        assert_eq!(inst.pubkey, s2.pubkey);
        assert_eq!(inst.signing_keys(&bk).public_key().to_hex(), s2.pubkey);
        assert_ne!(inst.signing_keys(&bk).public_key().to_hex(), bp);
    }

    #[test]
    fn different_rooms_allocate_distinct_global_ordinals() {
        // A live smith in #a and another live smith in #b must not share a
        // pubkey. Channels are membership scopes, not identity scopes.
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let a = select_and_reserve(&mut r, &mut sk, request("a", "#a", &bp, &bk, None)).unwrap();
        let b = select_and_reserve(&mut r, &mut sk, request("b", "#b", &bp, &bk, None)).unwrap();
        assert_eq!(a.ordinal, 0);
        assert_eq!(b.ordinal, 1);
        assert_ne!(a.pubkey, b.pubkey);
    }

    #[test]
    fn lowest_free_fills_gap_after_release() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        select_and_reserve(&mut r, &mut sk, request("s0", "#a", &bp, &bk, None)).unwrap();
        select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        // Release ordinal 0; next allocation refills the gap at 0.
        release(&mut r, &mut sk, "s0");
        let s2 = select_and_reserve(&mut r, &mut sk, request("s2", "#a", &bp, &bk, None)).unwrap();
        assert_eq!(s2.ordinal, 0);
    }

    #[test]
    fn channel_roster_occupancy_blocks_reusing_released_ordinal() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut occupied = HashSet::new();
        occupied.insert(bp.clone());
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let s = select_and_reserve(
            &mut r,
            &mut sk,
            SignerRequest {
                session_id: "s1",
                base_pubkey: &bp,
                agent_slug: "smith",
                h: "#a",
                base_keys: &bk,
                preferred_ordinal: None,
                occupied_pubkeys: Some(&occupied),
                owned_pubkey: None,
            },
        )
        .unwrap();
        assert_eq!(s.ordinal, 1);
        assert_eq!(s.label, "smith1");
    }

    #[test]
    fn existing_session_can_reassert_its_roster_pubkey() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut occupied = HashSet::new();
        occupied.insert(bp.clone());
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let s = select_and_reserve(
            &mut r,
            &mut sk,
            SignerRequest {
                session_id: "s1",
                base_pubkey: &bp,
                agent_slug: "smith",
                h: "#a",
                base_keys: &bk,
                preferred_ordinal: Some(0),
                occupied_pubkeys: Some(&occupied),
                owned_pubkey: Some(&bp),
            },
        )
        .unwrap();
        assert_eq!(s.ordinal, 0);
    }

    #[test]
    fn preferred_ordinal_is_honored_when_free() {
        // Mention-driven / resume: honor the exact ordinal even if lower ones are
        // free (a mention to smith1 in an empty room still spawns smith1).
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let s =
            select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, Some(1))).unwrap();
        assert_eq!(s.ordinal, 1);
        assert_eq!(s.label, "smith1");
    }

    #[test]
    fn reassert_keeps_same_ordinal() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        select_and_reserve(&mut r, &mut sk, request("s0", "#a", &bp, &bk, None)).unwrap();
        let s1a = select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        // Reassert s1 (e.g. a second session_start hook) → same ordinal, no new slot.
        let s1b = select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        assert_eq!(s1a.ordinal, s1b.ordinal);
        assert_eq!(s1a.pubkey, s1b.pubkey);
        assert_eq!(r.len(), 2); // s0 + s1, no duplicate slot
    }

    #[test]
    fn different_agents_independent_ordinals() {
        let smith = base_keys();
        let jones = Keys::new(nostr_sdk::prelude::SecretKey::from_slice(&[0x33; 32]).unwrap());
        let sp = smith.public_key().to_hex();
        let jp = jones.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let a = select_and_reserve(
            &mut r,
            &mut sk,
            SignerRequest {
                session_id: "s1",
                base_pubkey: &sp,
                agent_slug: "smith",
                h: "#a",
                base_keys: &smith,
                preferred_ordinal: None,
                occupied_pubkeys: None,
                owned_pubkey: None,
            },
        )
        .unwrap();
        let b = select_and_reserve(
            &mut r,
            &mut sk,
            SignerRequest {
                session_id: "s2",
                base_pubkey: &jp,
                agent_slug: "jones",
                h: "#a",
                base_keys: &jones,
                preferred_ordinal: None,
                occupied_pubkeys: None,
                owned_pubkey: None,
            },
        )
        .unwrap();
        // Both get ordinal 0 in the same room — independent per agent.
        assert_eq!(a.ordinal, 0);
        assert_eq!(b.ordinal, 0);
        assert_ne!(a.pubkey, b.pubkey);
    }

    #[test]
    fn reservation_mutex_shape_prevents_two_same_ordinal_winners() {
        use std::sync::{Arc, Mutex};
        use std::thread;
        let bk = Arc::new(base_keys());
        let bp = Arc::new(bk.public_key().to_hex());
        let reservations = Arc::new(Mutex::new(SignerReservations::new()));
        let session_keys = Arc::new(Mutex::new(HashMap::new()));
        let ordinals = Arc::new(Mutex::new(Vec::new()));
        thread::scope(|scope| {
            for sid in ["s1", "s2"] {
                let reservations = Arc::clone(&reservations);
                let session_keys = Arc::clone(&session_keys);
                let ordinals = Arc::clone(&ordinals);
                let bk = Arc::clone(&bk);
                let bp = Arc::clone(&bp);
                scope.spawn(move || {
                    let mut r = reservations.lock().unwrap();
                    let mut sk = session_keys.lock().unwrap();
                    let s = select_and_reserve(&mut r, &mut sk, request(sid, "#a", &bp, &bk, None))
                        .unwrap();
                    ordinals.lock().unwrap().push(s.ordinal);
                });
            }
        });
        let mut got = ordinals.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, vec![0, 1]); // two distinct ordinals, never both 0
    }
}
