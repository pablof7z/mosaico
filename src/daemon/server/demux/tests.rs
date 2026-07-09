use super::*;
use crate::state::session_claims::SessionClaim;
use crate::state::{Identity, RegisterSession, Store};

// ── helpers ───────────────────────────────────────────────────────────────────

fn register(store: &Store, pubkey: &str, slug: &str, channel: &str, external_id: &str) -> String {
    store
        .register_session(&RegisterSession {
            harness: "claude-code".into(),
            external_id_kind: "harness_session".into(),
            external_id: external_id.into(),
            agent_pubkey: pubkey.into(),
            agent_slug: slug.into(),
            channel_h: channel.into(),
            child_pid: Some(42),
            transcript_path: None,
            resume_id: String::new(),
            now: 1000,
        })
        .unwrap()
}

fn claim(pubkey: &str, channel: &str, owner_backend: &str, expires_at: u64) -> SessionClaim {
    SessionClaim {
        pubkey: pubkey.to_string(),
        agent_slug: "codex".to_string(),
        codename: "willow-echo-042".to_string(),
        session_id: "sid".to_string(),
        channel_h: channel.to_string(),
        native_id: "native-1".to_string(),
        harness: "codex".to_string(),
        last_active_at: 10,
        expires_at,
        owner_backend_pubkey: owner_backend.to_string(),
        owner_host: "laptop".to_string(),
    }
}

fn identity(pubkey: &str, codename: &str, slug: &str, channel: &str) -> Identity {
    Identity {
        pubkey: pubkey.to_string(),
        agent_slug: slug.to_string(),
        codename: codename.to_string(),
        session_id: format!("sid-{channel}"),
        channel_h: channel.to_string(),
        native_id: "native".to_string(),
        alive: false,
        created_at: 1,
    }
}

// ── has_alive gate ────────────────────────────────────────────────────────────

/// `handle` returns early when the mentioned pubkey already has an alive session
/// joined to the channel. This replicates the exact store query.
fn has_alive_session_for(store: &Store, mentioned_pk: &str, project: &str) -> bool {
    store
        .list_alive_sessions()
        .unwrap_or_default()
        .into_iter()
        .any(|rec| {
            rec.agent_pubkey == mentioned_pk
                && store
                    .is_session_joined_channel(&rec.session_id, project)
                    .unwrap_or(rec.channel_h == project)
        })
}

#[test]
fn has_alive_gate_skips_when_agent_has_live_session_in_channel() {
    let store = Store::open_memory().unwrap();
    let sid = register(&store, "pk-ord-1", "codex", "proj", "ext-1");
    // register_session with child_pid=Some sets alive=1.
    assert!(!sid.is_empty());

    assert!(has_alive_session_for(&store, "pk-ord-1", "proj"));
}

#[test]
fn has_alive_gate_does_not_skip_when_session_is_dead() {
    let store = Store::open_memory().unwrap();
    let sid = register(&store, "pk-ord-1", "codex", "proj", "ext-1");
    store.mark_dead(&sid).unwrap();

    assert!(!has_alive_session_for(&store, "pk-ord-1", "proj"));
}

#[test]
fn has_alive_gate_does_not_skip_when_agent_in_different_channel() {
    let store = Store::open_memory().unwrap();
    let _sid = register(&store, "pk-ord-1", "codex", "other-proj", "ext-1");

    assert!(!has_alive_session_for(&store, "pk-ord-1", "proj"));
}

#[test]
fn has_alive_gate_matches_derived_ordinal_pubkey_not_base() {
    let store = Store::open_memory().unwrap();
    // Session registered with the ordinal pubkey, not the base.
    let _sid = register(&store, "pk-ord-2", "codex", "proj", "ext-2");

    assert!(has_alive_session_for(&store, "pk-ord-2", "proj"));
    assert!(!has_alive_session_for(&store, "base-pk", "proj"));
}

#[test]
fn has_alive_gate_matches_joined_subchannel_not_just_home_channel() {
    let store = Store::open_memory().unwrap();
    let sid = register(&store, "pk-ord-1", "codex", "proj", "ext-1");
    // Join a sub-channel
    store.join_session_channel(&sid, "sub-chan", 10).unwrap();

    assert!(has_alive_session_for(&store, "pk-ord-1", "sub-chan"));
}

// ── remote backend claim gate ─────────────────────────────────────────────────

const BACKEND_A: &str = "backend-a-pubkey";
const BACKEND_B: &str = "backend-b-pubkey";

/// Replicates the remote-claim check: if the active (non-expired) claim belongs
/// to a different backend, skip the spawn.
fn remote_backend_owns_active_claim(
    store: &Store,
    mentioned_pk: &str,
    project: &str,
    now: u64,
    our_backend: Option<&str>,
) -> bool {
    store
        .get_active_session_claim(mentioned_pk, project, now)
        .ok()
        .flatten()
        .as_ref()
        .filter(|c| !c.is_owned_by_backend(our_backend))
        .is_some()
}

#[test]
fn remote_claim_gate_skips_when_active_claim_owned_by_other_backend() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_session_claim(&claim("pk", "proj", BACKEND_B, 100))
        .unwrap();

    assert!(remote_backend_owns_active_claim(
        &store,
        "pk",
        "proj",
        50,
        Some(BACKEND_A)
    ));
}

#[test]
fn remote_claim_gate_does_not_skip_when_active_claim_owned_by_us() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_session_claim(&claim("pk", "proj", BACKEND_A, 100))
        .unwrap();

    assert!(!remote_backend_owns_active_claim(
        &store,
        "pk",
        "proj",
        50,
        Some(BACKEND_A)
    ));
}

#[test]
fn remote_claim_gate_does_not_skip_when_claim_expired() {
    let store = Store::open_memory().unwrap();
    // Claim expired at t=10; now=50.
    store
        .upsert_session_claim(&claim("pk", "proj", BACKEND_B, 10))
        .unwrap();

    assert!(!remote_backend_owns_active_claim(
        &store,
        "pk",
        "proj",
        50,
        Some(BACKEND_A)
    ));
}

#[test]
fn remote_claim_gate_treats_legacy_empty_owner_as_local() {
    let store = Store::open_memory().unwrap();
    let mut c = claim("pk", "proj", "", 100);
    c.owner_backend_pubkey.clear();
    store.upsert_session_claim(&c).unwrap();

    // Empty owner is treated as "ours" regardless of our backend pubkey.
    assert!(!remote_backend_owns_active_claim(
        &store,
        "pk",
        "proj",
        50,
        Some(BACKEND_A)
    ));
    assert!(!remote_backend_owns_active_claim(
        &store, "pk", "proj", 50, None
    ));
}

#[test]
fn remote_claim_gate_skips_when_we_have_no_backend_pubkey_and_claim_has_owner() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_session_claim(&claim("pk", "proj", BACKEND_A, 100))
        .unwrap();

    // Our backend key is None — we can't prove ownership, so a claim with a
    // real owner is treated as "not ours" → remote → skip.
    assert!(remote_backend_owns_active_claim(
        &store, "pk", "proj", 50, None
    ));
}

// ── identity resolution ───────────────────────────────────────────────────────

/// Replicates the identity resolution path in `handle`: try channel-specific
/// first, then global. Returns (slug, native_id) or None. A per-session pubkey is
/// unique, so this resolves to exactly one session whose native id can be resumed.
fn resolve_identity(store: &Store, mentioned_pk: &str, project: &str) -> Option<(String, String)> {
    store
        .get_identity_for_channel(mentioned_pk, project)
        .ok()
        .flatten()
        .or_else(|| store.get_identity(mentioned_pk).ok().flatten())
        .map(|idn| (idn.agent_slug, idn.native_id))
}

#[test]
fn identity_resolution_prefers_channel_specific_identity() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_identity(&identity("pk", "willow-echo-042", "codex", "proj"))
        .unwrap();
    store
        .upsert_identity(&identity("pk2", "cedar-orbit-113", "claude", "other"))
        .unwrap();

    let (slug, native_id) = resolve_identity(&store, "pk", "proj").unwrap();
    assert_eq!(slug, "codex");
    assert_eq!(native_id, "native");
}

#[test]
fn identity_resolution_falls_back_to_global_when_no_channel_match() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_identity(&identity("pk", "cedar-orbit-113", "claude", "other"))
        .unwrap();

    let (slug, _) = resolve_identity(&store, "pk", "proj").unwrap();
    assert_eq!(slug, "claude");
}

#[test]
fn identity_resolution_returns_none_for_unknown_pubkey() {
    let store = Store::open_memory().unwrap();
    assert!(resolve_identity(&store, "unknown-pk", "proj").is_none());
}

// ── resume decision ───────────────────────────────────────────────────────────

/// Replicates the resume decision in `handle`: resume the exact session (which
/// reproduces the p-tagged pubkey) when we know its native id, else fresh spawn.
fn should_resume(native_id: &str) -> bool {
    !native_id.is_empty()
}

#[test]
fn resumes_when_native_id_known() {
    assert!(should_resume("native-claude-1"));
}

#[test]
fn fresh_spawn_when_native_id_unknown() {
    assert!(!should_resume(""));
}

// ── eye-reaction routing gate ─────────────────────────────────────────────────

/// Replicates the `hosted.contains(mentioned_pk)` gate in handle_incoming that
/// decides whether to publish the eye reaction.
fn should_publish_eye_reaction(hosted: &[String], mentioned_pk: &str) -> bool {
    hosted.contains(&mentioned_pk.to_string())
}

#[test]
fn eye_reaction_fires_for_hosted_agent_pubkey() {
    let hosted = vec!["pk-ord-1".to_string(), "pk-ord-2".to_string()];
    assert!(should_publish_eye_reaction(&hosted, "pk-ord-1"));
}

#[test]
fn eye_reaction_fires_for_identity_derived_pubkey() {
    // The hosted set in handle_incoming extends with list_identity_pubkeys(),
    // so derived ordinal pubkeys are recognized as local.
    let hosted = vec!["base-pk".to_string(), "pk-ord-1".to_string()];
    assert!(should_publish_eye_reaction(&hosted, "pk-ord-1"));
}

#[test]
fn eye_reaction_does_not_fire_for_foreign_peer() {
    let hosted = vec!["pk-ord-1".to_string()];
    assert!(!should_publish_eye_reaction(&hosted, "foreign-pk"));
}

#[test]
fn eye_reaction_does_not_fire_for_empty_mentioned_pk() {
    let hosted = vec!["pk-ord-1".to_string()];
    assert!(!should_publish_eye_reaction(&hosted, ""));
}

// ── proactive-warm selection (existing test kept) ─────────────────────────────

/// The proactive-warm selection: already-named identities are skipped (no
/// network), empty pubkeys are ignored, and a pubkey already in flight is not
/// re-claimed, so duplicate relay deliveries collapse to one fetch.
#[tokio::test]
async fn claim_skips_known_empty_and_in_flight() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|s| {
        s.upsert_profile("known-pk", "pablo", "pablo", "laptop", false, 1)
            .unwrap();
    });

    let claimed = claim_pubkeys_to_warm(
        &state,
        vec!["known-pk".into(), "new-pk".into(), String::new()],
    );
    assert_eq!(
        claimed,
        vec!["new-pk".to_string()],
        "only the uncached, non-empty pubkey is claimed for a fetch"
    );

    let again = claim_pubkeys_to_warm(&state, vec!["new-pk".into()]);
    assert!(again.is_empty(), "an in-flight pubkey is not re-claimed");
}
