use crate::identity;
use anyhow::Result;
use nostr_sdk::prelude::Keys;
use std::collections::{HashMap, HashSet};

/// One live session per `(derivation root, channel, ordinal)`.
/// The same ordinal key may be reused concurrently in different channels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct OrdinalSlot {
    base_pubkey: String,
    channel_h: String,
    ordinal: u32,
}

/// In-memory reservation map: slot → owning session id.
pub(super) type SignerReservations = HashMap<OrdinalSlot, String>;

pub(super) struct SignerRequest<'a> {
    pub session_id: &'a str,
    /// Local derivation-root pubkey for this capability.
    pub base_pubkey: &'a str,
    pub agent_slug: &'a str,
    /// The NIP-29 group id the session operates in (`route_scope`).
    pub h: &'a str,
    /// Local keypair used only as the HKDF derivation root for ordinal keys.
    pub base_keys: &'a Keys,
    /// Honor this exact ordinal when free (resume of a known route, or a
    /// mention-driven spawn naming a specific `smithN`). `None` → allocate the
    /// lowest free channel ordinal (birth).
    pub preferred_ordinal: Option<u32>,
    /// Pubkeys currently unavailable for reuse in the target channel.
    pub occupied_pubkeys: Option<&'a HashSet<String>>,
    /// Pubkey this same session or exact preferred route is allowed to reuse.
    /// Generic births still treat roster membership as occupied; mention-driven
    /// exact routes must keep the pubkey that was p-tagged so replay can deliver.
    pub owned_pubkey: Option<&'a str>,
}

/// The ordinal identity selected for a session.
pub(super) struct SelectedSigner {
    pub ordinal: u32,
    pub pubkey: String,
    pub label: String,
    keys: Keys,
}

impl SelectedSigner {
    /// Project to the authoritative [`crate::identity::AgentInstance`] (issue #98).
    /// The base slug/pubkey identify the local derivation family; this signer
    /// contributes the selected ordinal + pubkey. Engine + publishers derive
    /// label/pubkey/signing-key policy from the returned instance, never from
    /// the raw `label`/`keys` fields here.
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
}

fn slot(base_pubkey: &str, channel_h: &str, ordinal: u32) -> OrdinalSlot {
    OrdinalSlot {
        base_pubkey: base_pubkey.to_string(),
        channel_h: channel_h.to_string(),
        ordinal,
    }
}

/// Construct the concrete signer for an ordinal: derive its keypair, label, and
/// pubkey.
fn build(req: &SignerRequest<'_>, ordinal: u32) -> SelectedSigner {
    let keys = identity::derive_agent_ordinal_keys(req.base_keys, ordinal);
    let pubkey = keys.public_key().to_hex();
    let label = identity::agent_ordinal_label(req.agent_slug, ordinal);
    SelectedSigner {
        ordinal,
        pubkey,
        label,
        keys,
    }
}

/// Commit a chosen ordinal: record its engine keys and return the signer. The
/// reservation must already be inserted.
fn finish(
    session_keys: &mut HashMap<String, Keys>,
    req: &SignerRequest<'_>,
    ordinal: u32,
) -> SelectedSigner {
    let signer = build(req, ordinal);
    session_keys.insert(req.session_id.to_string(), signer.keys.clone());
    signer
}

fn is_taken(r: &SignerReservations, req: &SignerRequest<'_>, ordinal: u32) -> bool {
    if r.get(&slot(req.base_pubkey, req.h, ordinal))
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

/// Lowest ordinal not currently reserved by a live same-channel session or
/// otherwise visible as occupied in the target channel.
fn lowest_free(r: &SignerReservations, req: &SignerRequest<'_>) -> u32 {
    let mut n = 1u32;
    while is_taken(r, req, n) {
        n += 1;
    }
    n
}

/// Select and reserve a channel-scoped ordinal identity for a session.
///
/// - Reassert: if this session already owns a slot, keep its ordinal.
/// - `preferred_ordinal` (resume/mention): honor it when free.
/// - Otherwise allocate the lowest free channel ordinal (birth).
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

    let ordinal = match req.preferred_ordinal.filter(|n| *n > 0) {
        Some(n) if !is_taken(reservations, &req, n) => n,
        Some(preferred) => {
            anyhow::bail!(
                "preferred ordinal {preferred} for {} is already active in channel {}",
                req.agent_slug,
                req.h
            );
        }
        None => lowest_free(reservations, &req),
    };
    reservations.insert(
        slot(req.base_pubkey, req.h, ordinal),
        req.session_id.to_string(),
    );
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

/// Move an existing live session reservation to a new channel. This keeps the
/// channel-scoped collision guard aligned with `sessions.channel_h` after
/// `channels switch` / create auto-focus.
pub(super) fn move_channel(
    reservations: &mut SignerReservations,
    session_id: &str,
    new_channel: &str,
) -> Result<()> {
    let Some((old_slot, owner)) = reservations
        .iter()
        .find(|(_, owner)| owner.as_str() == session_id)
        .map(|(slot, owner)| (slot.clone(), owner.clone()))
    else {
        return Ok(());
    };
    if old_slot.channel_h == new_channel {
        return Ok(());
    }
    let new_slot = slot(&old_slot.base_pubkey, new_channel, old_slot.ordinal);
    if reservations
        .get(&new_slot)
        .is_some_and(|existing| existing != session_id)
    {
        anyhow::bail!(
            "ordinal {} for this agent is already reserved in channel {}",
            old_slot.ordinal,
            new_channel
        );
    }
    reservations.remove(&old_slot);
    reservations.insert(new_slot, owner);
    Ok(())
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
    fn first_session_in_room_is_ordinal_one_derived_key() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let s = select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        assert_eq!(s.ordinal, 1);
        assert_eq!(s.label, "smith1");
        assert_ne!(s.pubkey, bp);
        assert!(sk.contains_key("s1"));
        let inst = s.instance("smith", &bp);
        assert_eq!(inst.display_slug(), "smith1");
        assert_eq!(inst.pubkey, s.pubkey);
        assert_eq!(inst.signing_keys(&bk).public_key().to_hex(), s.pubkey);
    }

    #[test]
    fn second_same_agent_same_room_is_ordinal_two() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        let s2 = select_and_reserve(&mut r, &mut sk, request("s2", "#a", &bp, &bk, None)).unwrap();
        assert_eq!(s2.ordinal, 2);
        assert_eq!(s2.label, "smith2");
        assert_ne!(s2.pubkey, bp);
        assert_eq!(
            s2.pubkey,
            identity::derive_agent_ordinal_keys(&bk, 2)
                .public_key()
                .to_hex()
        );
        let inst = s2.instance("smith", &bp);
        assert_eq!(inst.display_slug(), "smith2");
        assert_eq!(inst.pubkey, s2.pubkey);
        assert_eq!(inst.signing_keys(&bk).public_key().to_hex(), s2.pubkey);
        assert_ne!(inst.signing_keys(&bk).public_key().to_hex(), bp);
    }

    #[test]
    fn different_rooms_can_reuse_same_ordinal_key() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        let a = select_and_reserve(&mut r, &mut sk, request("a", "#a", &bp, &bk, None)).unwrap();
        let b = select_and_reserve(&mut r, &mut sk, request("b", "#b", &bp, &bk, None)).unwrap();
        assert_eq!(a.ordinal, 1);
        assert_eq!(b.ordinal, 1);
        assert_eq!(a.pubkey, b.pubkey);
    }

    #[test]
    fn lowest_free_fills_gap_after_release() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let mut r = SignerReservations::new();
        let mut sk = HashMap::new();
        select_and_reserve(&mut r, &mut sk, request("s0", "#a", &bp, &bk, None)).unwrap();
        select_and_reserve(&mut r, &mut sk, request("s1", "#a", &bp, &bk, None)).unwrap();
        // Release ordinal 1; next allocation refills the gap at 1.
        release(&mut r, &mut sk, "s0");
        let s2 = select_and_reserve(&mut r, &mut sk, request("s2", "#a", &bp, &bk, None)).unwrap();
        assert_eq!(s2.ordinal, 1);
    }

    #[test]
    fn channel_roster_occupancy_blocks_reusing_released_ordinal() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let ord1_pubkey = identity::derive_agent_ordinal_keys(&bk, 1)
            .public_key()
            .to_hex();
        let mut occupied = HashSet::new();
        occupied.insert(ord1_pubkey.clone());
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
        assert_eq!(s.ordinal, 2);
        assert_eq!(s.label, "smith2");
    }

    #[test]
    fn existing_session_can_reassert_its_roster_pubkey() {
        let bk = base_keys();
        let bp = bk.public_key().to_hex();
        let ord1_pubkey = identity::derive_agent_ordinal_keys(&bk, 1)
            .public_key()
            .to_hex();
        let mut occupied = HashSet::new();
        occupied.insert(ord1_pubkey.clone());
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
                preferred_ordinal: Some(1),
                occupied_pubkeys: Some(&occupied),
                owned_pubkey: Some(ord1_pubkey.as_str()),
            },
        )
        .unwrap();
        assert_eq!(s.ordinal, 1);
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
        // Both get ordinal 1 in the same room — independent per agent family.
        assert_eq!(a.ordinal, 1);
        assert_eq!(b.ordinal, 1);
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
        assert_eq!(got, vec![1, 2]); // two distinct same-channel ordinals
    }
}
