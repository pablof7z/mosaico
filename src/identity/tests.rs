use super::*;

mod byline;
mod commands;

#[test]
fn generates_then_reloads_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let a = load_or_create(dir.path(), "coder", 100).unwrap();
    let b = load_or_create(dir.path(), "coder", 200).unwrap();
    assert_eq!(a.pubkey_hex(), b.pubkey_hex());
    assert_eq!(
        a.keys.secret_key().to_secret_hex(),
        b.keys.secret_key().to_secret_hex()
    );
}

#[test]
fn distinct_slugs_get_distinct_keys() {
    let dir = tempfile::tempdir().unwrap();
    let a = load_or_create(dir.path(), "coder", 1).unwrap();
    let b = load_or_create(dir.path(), "reviewer", 1).unwrap();
    assert_ne!(a.pubkey_hex(), b.pubkey_hex());
}

#[test]
fn rejects_bad_slug() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_or_create(dir.path(), "bad slug/with-stuff", 1).is_err());
    assert!(load_or_create(dir.path(), "", 1).is_err());
}

#[test]
fn persists_to_expected_path() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", 1).unwrap();
    assert!(dir.path().join("agents").join("coder.json").exists());
}

#[test]
fn add_local_agent_creates_then_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (a, created) = add_local_agent(dir.path(), "coder", None, 1).unwrap();
    assert!(created, "first add mints a fresh key");
    assert!(dir.path().join("agents").join("coder.json").exists());

    let (b, created2) = add_local_agent(dir.path(), "coder", None, 2).unwrap();
    assert!(!created2, "re-adding an existing slug does not recreate");
    assert_eq!(a.pubkey_hex(), b.pubkey_hex());
}

#[test]
fn remove_local_agent_parks_then_reports_missing() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", 1).unwrap();
    let live = dir.path().join("agents").join("coder.json");
    assert!(live.exists());

    let parked = remove_local_agent(dir.path(), "coder").unwrap();
    let parked = parked.expect("removing an existing agent returns the parked path");
    assert!(!live.exists(), "live key file is gone");
    assert!(parked.exists(), "key is parked, not unlinked");
    assert!(list_local_agent_details(dir.path()).is_empty());
    assert!(list_local_pubkeys(dir.path()).is_empty());

    assert!(remove_local_agent(dir.path(), "coder").unwrap().is_none());
}

fn test_tenex_secret() -> SecretKey {
    SecretKey::from_slice(&[0x01u8; 32]).unwrap()
}

#[test]
fn session_key_determinism() {
    let sk = test_tenex_secret();
    let a = derive_session_keys(&sk, "my-project", "coder", "claude", "sess-abc");
    let b = derive_session_keys(&sk, "my-project", "coder", "claude", "sess-abc");
    assert_eq!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "derive_session_keys must be deterministic"
    );
    assert_eq!(
        a.secret_key().to_secret_hex(),
        b.secret_key().to_secret_hex(),
    );
}

#[test]
fn session_key_different_anchors_differ() {
    let sk = test_tenex_secret();
    let a = derive_session_keys(&sk, "proj", "coder", "claude", "session-1");
    let b = derive_session_keys(&sk, "proj", "coder", "claude", "session-2");
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "different anchors must yield different session pubkeys"
    );
}

#[test]
fn session_key_different_projects_differ() {
    let sk = test_tenex_secret();
    let a = derive_session_keys(&sk, "project-alpha", "coder", "claude", "anchor-x");
    let b = derive_session_keys(&sk, "project-beta", "coder", "claude", "anchor-x");
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "different project slugs must yield different session pubkeys"
    );
}

#[test]
fn session_key_different_agent_slugs_differ() {
    let sk = test_tenex_secret();
    let a = derive_session_keys(&sk, "proj", "coder", "claude", "anchor");
    let b = derive_session_keys(&sk, "proj", "reviewer", "claude", "anchor");
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "different agent slugs must yield different session pubkeys"
    );
}

#[test]
fn session_key_field_boundary_non_collision() {
    let sk = test_tenex_secret();
    let a = derive_session_keys(&sk, "a", "bc", "claude", "anchor");
    let b = derive_session_keys(&sk, "ab", "c", "claude", "anchor");
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "field-boundary collision: (project='a', agent='bc') must differ from (project='ab', agent='c')"
    );
}

#[test]
fn session_key_known_answer() {
    let sk = test_tenex_secret();
    let keys = derive_session_keys(&sk, "my-project", "coder", "claude", "sess-abc");
    assert_eq!(
        keys.public_key().to_hex(),
        "9aa6883eee2f1ce43053a1eec2c1c8b1c712cbb3c77ec346d9f091982a50b461",
        "known-answer test: pinned pubkey changed"
    );
}

#[test]
fn ac1_resumed_session_derives_same_pubkey() {
    let sk = test_tenex_secret();
    let harness_id = "claude-native-xKz8-resume-test";

    let original = derive_session_keys(&sk, "my-project", "coder", "claude", harness_id);
    let resumed = derive_session_keys(&sk, "my-project", "coder", "claude", harness_id);

    assert_eq!(
        original.public_key().to_hex(),
        resumed.public_key().to_hex(),
        "AC1: a resumed harness session must reproduce the exact same session pubkey"
    );
    assert_eq!(
        original.secret_key().to_secret_hex(),
        resumed.secret_key().to_secret_hex(),
        "AC1: and the exact same secret key"
    );
}

#[test]
fn ac2_two_sessions_same_agent_different_pubkeys() {
    let sk = test_tenex_secret();

    let session_a = derive_session_keys(&sk, "proj", "coder", "claude", "native-id-aaaa");
    let session_b = derive_session_keys(&sk, "proj", "coder", "claude", "native-id-bbbb");

    assert_ne!(
        session_a.public_key().to_hex(),
        session_b.public_key().to_hex(),
        "AC2: two distinct harness sessions for the same agent must have different pubkeys"
    );
}

#[test]
fn ac3_same_harness_id_different_projects_isolate() {
    let sk = test_tenex_secret();
    let anchor = "same-harness-id-across-projects";

    let proj_alpha = derive_session_keys(&sk, "project-alpha", "coder", "claude", anchor);
    let proj_beta = derive_session_keys(&sk, "project-beta", "coder", "claude", anchor);

    assert_ne!(
        proj_alpha.public_key().to_hex(),
        proj_beta.public_key().to_hex(),
        "AC3: same harness id must yield different session pubkeys in different projects"
    );
}

// -----------------------------------------------------------------------
// derive_agent_ordinal_keys tests (issue #47)
// -----------------------------------------------------------------------

#[test]
fn ordinal_zero_is_a_derived_legacy_key() {
    let base = Keys::generate();
    let zero = derive_agent_ordinal_keys(&base, 0);
    assert_ne!(zero.public_key().to_hex(), base.public_key().to_hex());
    assert_ne!(
        zero.secret_key().to_secret_hex(),
        base.secret_key().to_secret_hex()
    );
}

#[test]
fn ordinal_derivation_is_deterministic_and_room_independent() {
    // The function takes no room/project input, so determinism alone proves
    // room-independence: smith1 is the same pubkey wherever it is used.
    let base = Keys::new(test_tenex_secret());
    let a = derive_agent_ordinal_keys(&base, 1);
    let b = derive_agent_ordinal_keys(&base, 1);
    assert_eq!(a.public_key().to_hex(), b.public_key().to_hex());
    assert_eq!(
        a.secret_key().to_secret_hex(),
        b.secret_key().to_secret_hex()
    );
}

#[test]
fn distinct_ordinals_get_distinct_keys() {
    let base = Keys::new(test_tenex_secret());
    let zero = derive_agent_ordinal_keys(&base, 0).public_key().to_hex();
    let one = derive_agent_ordinal_keys(&base, 1).public_key().to_hex();
    let two = derive_agent_ordinal_keys(&base, 2).public_key().to_hex();
    assert_ne!(zero, one);
    assert_ne!(one, two);
    assert_ne!(zero, two);
}

#[test]
fn distinct_base_agents_get_distinct_ordinal_families() {
    // smith1 and jones1 must differ — the base secret keys the family.
    let smith = Keys::generate();
    let jones = Keys::generate();
    assert_ne!(
        derive_agent_ordinal_keys(&smith, 1).public_key().to_hex(),
        derive_agent_ordinal_keys(&jones, 1).public_key().to_hex()
    );
}

#[test]
fn ordinal_known_answer() {
    // Pinned known-answer: base secret = [0x01;32], ordinal 1.
    // Catches any change to the derivation spec; bump the salt version if
    // the encoding ever changes intentionally.
    let base = Keys::new(test_tenex_secret());
    let k = derive_agent_ordinal_keys(&base, 1);
    assert_eq!(
        k.public_key().to_hex(),
        "90ea491a638c1b58f05fd81b4005852aac8defccc1bbd473231fcbfa24589804",
        "known-answer test: pinned ordinal-1 pubkey changed — was the derivation spec modified?"
    );
}

#[test]
fn ordinal_label_format() {
    assert_eq!(agent_ordinal_label("smith", 0), "smith0");
    assert_eq!(agent_ordinal_label("smith", 1), "smith1");
    assert_eq!(agent_ordinal_label("smith", 12), "smith12");
}

// ── AgentInstance (issue #98) ─────────────────────────────────────────────────

#[test]
fn agent_instance_ordinal_zero_is_legacy_labeled() {
    let base = Keys::new(SecretKey::from_slice(&[0x11; 32]).unwrap());
    let bp = base.public_key().to_hex();
    let inst = AgentInstance::from_parts("claude".into(), bp.clone(), 0, bp.clone());

    assert_eq!(inst.display_slug(), "claude0");
    assert_eq!(inst.pubkey, bp);
    assert_eq!(
        inst.signing_keys(&base).secret_key().to_secret_hex(),
        base.secret_key().to_secret_hex()
    );
    let aref = inst.agent_ref();
    assert_eq!(aref.pubkey, bp);
    assert_eq!(aref.slug, "claude0");
}

#[test]
fn agent_instance_ordinal_n_is_internally_consistent() {
    let base = Keys::new(SecretKey::from_slice(&[0x11; 32]).unwrap());
    let bp = base.public_key().to_hex();
    let ord_keys = derive_agent_ordinal_keys(&base, 1);
    let ord_pubkey = ord_keys.public_key().to_hex();
    let inst = AgentInstance::from_parts("claude".into(), bp.clone(), 1, ord_pubkey.clone());

    // Label is the ordinal-qualified name; selected pubkey is the derived key.
    assert_eq!(inst.display_slug(), "claude1");
    assert_ne!(inst.pubkey, bp);
    assert_eq!(inst.pubkey, ord_pubkey);
    // Signing keys derive to the SAME ordinal pubkey (no base collapse).
    assert_eq!(inst.signing_keys(&base).public_key().to_hex(), ord_pubkey);
    // The wire identity never mixes the ordinal pubkey with the bare slug.
    let aref = inst.agent_ref();
    assert_eq!(aref.pubkey, ord_pubkey);
    assert_eq!(aref.slug, "claude1");
}

#[test]
fn agent_instance_base_fallback_displays_first_ordinal() {
    let inst = AgentInstance::base("claude".into(), "deadbeef".into());
    assert_eq!(inst.ordinal, 1);
    assert_eq!(inst.pubkey, "deadbeef");
    assert_eq!(inst.base_pubkey, "deadbeef");
    assert_eq!(inst.display_slug(), "claude1");
}
