use super::*;

// ── ordinal identity + route tests (issue #47) ─────────────────────────────

fn route(pubkey: &str, h: &str, session: &str, ordinal: u32, alive: bool) -> IdentityRoute {
    IdentityRoute {
        pubkey: pubkey.into(),
        h: h.into(),
        session_id: session.into(),
        base_pubkey: "base-smith".into(),
        agent_slug: "smith".into(),
        ordinal,
        label: crate::identity::agent_ordinal_label("smith", ordinal),
        harness_kind: "claude-code".into(),
        native_id: format!("native-{session}"),
        alive,
    }
}

#[test]
fn ordinal_inventory_roundtrip() {
    let s = Store::open_memory().unwrap();
    s.ensure_agent_ordinal("base-smith", "smith", 1, "pk-smith1", 10)
        .unwrap();
    s.ensure_agent_ordinal("base-smith", "smith", 2, "pk-smith2", 10)
        .unwrap();
    // Idempotent on (base, ordinal).
    s.ensure_agent_ordinal("base-smith", "smith", 1, "pk-smith1", 20)
        .unwrap();
    let mut pks = s.list_agent_ordinal_pubkeys();
    pks.sort();
    assert_eq!(pks, vec!["pk-smith1", "pk-smith2"]);
    assert_eq!(
        s.local_agent_ordinal_for_pubkey("pk-smith1"),
        Some(("base-smith".into(), "smith".into(), 1))
    );
    assert_eq!(s.local_agent_ordinal_for_pubkey("nope"), None);
}

#[test]
fn identity_route_same_pubkey_two_rooms() {
    // The same ordinal pubkey is live in two rooms — one row per (pubkey, h),
    // disambiguated by h. This is the core (pubkey, h) routing invariant.
    let s = Store::open_memory().unwrap();
    s.upsert_identity_route(&route("pk-smith1", "#a", "sess-a", 1, true), 100)
        .unwrap();
    s.upsert_identity_route(&route("pk-smith1", "#b", "sess-b", 1, true), 100)
        .unwrap();
    assert_eq!(
        s.live_identity_route("pk-smith1", "#a").unwrap().session_id,
        "sess-a"
    );
    assert_eq!(
        s.live_identity_route("pk-smith1", "#b").unwrap().session_id,
        "sess-b"
    );
    assert_eq!(s.identity_route_for_session("sess-b").unwrap().h, "#b");
}

#[test]
fn lowest_free_ordinal_in_room() {
    let s = Store::open_memory().unwrap();
    // smith(0) and smith1 both live in #a.
    s.upsert_identity_route(&route("pk-smith0", "#a", "s0", 0, true), 100)
        .unwrap();
    s.upsert_identity_route(&route("pk-smith1", "#a", "s1", 1, true), 100)
        .unwrap();
    let mut live = s.live_ordinals_in_h("base-smith", "#a", None);
    live.sort();
    assert_eq!(live, vec![0, 1]);
    // #b is empty → ordinal 0 is free there (room-independent reuse).
    assert!(s.live_ordinals_in_h("base-smith", "#b", None).is_empty());
}

#[test]
fn dead_route_persists_for_resume_then_revives() {
    let s = Store::open_memory().unwrap();
    s.upsert_identity_route(&route("pk-smith1", "#a", "sess-old", 1, true), 100)
        .unwrap();
    s.mark_identity_route_dead("sess-old", 200).unwrap();
    // No live route, but the bound row survives with its native_id for resume.
    assert!(s.live_identity_route("pk-smith1", "#a").is_none());
    let bound = s.bound_identity_route("pk-smith1", "#a").unwrap();
    assert!(!bound.alive);
    assert_eq!(bound.native_id, "native-sess-old");
    // The ordinal is now free for allocation again.
    assert!(s.live_ordinals_in_h("base-smith", "#a", None).is_empty());
    // Resume: a new session rebinds (pubkey, h).
    s.upsert_identity_route(&route("pk-smith1", "#a", "sess-new", 1, true), 300)
        .unwrap();
    assert_eq!(
        s.live_identity_route("pk-smith1", "#a").unwrap().session_id,
        "sess-new"
    );
}

#[test]
fn move_route_on_channel_switch() {
    let s = Store::open_memory().unwrap();
    s.upsert_identity_route(&route("pk-smith1", "#origin", "sess-x", 1, true), 100)
        .unwrap();
    assert!(s.move_identity_route("sess-x", "#dst", 200).unwrap());
    // Old (pubkey, origin) is gone; new (pubkey, dst) is the resume key.
    assert!(s.live_identity_route("pk-smith1", "#origin").is_none());
    assert_eq!(
        s.live_identity_route("pk-smith1", "#dst").unwrap().session_id,
        "sess-x"
    );
}
