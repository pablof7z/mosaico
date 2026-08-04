use super::*;

fn at(pk: &str, slug: &str) -> AddTarget {
    AddTarget {
        backend_pubkey: pk.to_string(),
        slug: slug.to_string(),
        session_pubkey: None,
    }
}

fn resume(pk: &str, slug: &str, session_pubkey: &str) -> AddTarget {
    AddTarget {
        backend_pubkey: pk.to_string(),
        slug: slug.to_string(),
        session_pubkey: Some(session_pubkey.to_string()),
    }
}

fn sign(b: EventBuilder) -> Event {
    b.sign_with_keys(&Keys::generate()).unwrap()
}

fn tag_count(ev: &Event, name: &str) -> usize {
    ev.tags
        .iter()
        .filter(|t| t.as_slice().first().map(String::as_str) == Some(name))
        .count()
}

#[test]
fn build_parse_round_trip_preserves_order() {
    let adds = vec![
        at("bk1", "architect"),
        at("bk2", "engineer"),
        at("bk1", "qa"),
    ];
    let b = build_add_agents_event("parent-g", "child-g", &adds, "please add these").unwrap();
    let ev = sign(b);
    assert_eq!(ev.kind.as_u16(), KIND_CHAT);

    let op = parse_orchestration(&ev).expect("well-formed");
    assert_eq!(op.parent, "parent-g");
    assert_eq!(op.child_h, "child-g");
    assert_eq!(op.adds, adds, "add order preserved");
    assert!(!op.running_only);
}

#[test]
fn build_dedups_p_tags_but_keeps_all_adds() {
    let adds = vec![at("bk1", "architect"), at("bk1", "qa")];
    let ev = sign(build_add_agents_event("p", "c", &adds, "x").unwrap());
    // Two adds to the same backend → one p tag, two add tags.
    assert_eq!(tag_count(&ev, "p"), 1);
    assert_eq!(tag_count(&ev, "add"), 2);
}

#[test]
fn build_routes_h_to_parent_and_carries_child_in_h_target() {
    let ev = sign(build_add_agents_event("p", "c", &[at("bk", "r")], "x").unwrap());
    // Single routing h equals parent; child travels in h-target.
    assert_eq!(tag_count(&ev, "h"), 1);
    assert_eq!(tag_count(&ev, "h-target"), 1);
    let op = parse_orchestration(&ev).unwrap();
    assert_eq!(op.parent, "p");
    assert_eq!(op.child_h, "c");
}

#[test]
fn build_parse_preserves_optional_session_pubkey() {
    let adds = vec![resume("bk1", "architect", &"11".repeat(32))];
    let ev = sign(build_add_agents_event("p", "c", &adds, "x").unwrap());
    let op = parse_orchestration(&ev).unwrap();
    assert_eq!(op.adds, adds);
}

#[test]
fn build_parse_preserves_running_only_admission() {
    let adds = vec![resume("bk1", "architect", &"11".repeat(32))];
    let ev = sign(build_admit_running_event("p", "c", &adds, "x").unwrap());
    let op = parse_orchestration(&ev).unwrap();
    assert_eq!(op.adds, adds);
    assert!(op.running_only);
}

#[test]
fn admit_running_requires_an_exact_session() {
    let error = build_admit_running_event("p", "c", &[at("bk1", "architect")], "x")
        .expect_err("running admission without a session must fail");
    assert!(error.to_string().contains("exact session pubkeys"));
}

#[test]
fn parse_none_for_plain_chat_without_mosaico_op() {
    // A prose-only kind:9 chat message must be ignored.
    let ev = sign(
        EventBuilder::new(kind(KIND_CHAT), "just chatting")
            .tags([tag(&["h", "p"]).unwrap()])
            .allow_self_tagging(),
    );
    assert!(parse_orchestration(&ev).is_none());
}

#[test]
fn parse_none_for_different_mosaico_op() {
    let ev = sign(
        EventBuilder::new(kind(KIND_CHAT), "x")
            .tags([
                tag(&["h", "p"]).unwrap(),
                tag(&["mosaico-op", "subgroup.remove-agents.v1"]).unwrap(),
                tag(&["parent", "p"]).unwrap(),
                tag(&["h-target", "c"]).unwrap(),
                tag(&["add", "bk", "r"]).unwrap(),
            ])
            .allow_self_tagging(),
    );
    assert!(parse_orchestration(&ev).is_none());
}

#[test]
fn parse_none_when_h_differs_from_parent() {
    let ev = sign(
        EventBuilder::new(kind(KIND_CHAT), "x")
            .tags([
                tag(&["h", "other-group"]).unwrap(),
                tag(&["mosaico-op", MOSAICO_OP_ADD_AGENTS]).unwrap(),
                tag(&["parent", "p"]).unwrap(),
                tag(&["h-target", "c"]).unwrap(),
                tag(&["add", "bk", "r"]).unwrap(),
            ])
            .allow_self_tagging(),
    );
    assert!(parse_orchestration(&ev).is_none());
}

#[test]
fn parse_none_when_h_target_missing() {
    let ev = sign(
        EventBuilder::new(kind(KIND_CHAT), "x")
            .tags([
                tag(&["h", "p"]).unwrap(),
                tag(&["mosaico-op", MOSAICO_OP_ADD_AGENTS]).unwrap(),
                tag(&["parent", "p"]).unwrap(),
                tag(&["add", "bk", "r"]).unwrap(),
            ])
            .allow_self_tagging(),
    );
    assert!(parse_orchestration(&ev).is_none());
}

#[test]
fn parse_none_when_two_h_target_tags() {
    let ev = sign(
        EventBuilder::new(kind(KIND_CHAT), "x")
            .tags([
                tag(&["h", "p"]).unwrap(),
                tag(&["mosaico-op", MOSAICO_OP_ADD_AGENTS]).unwrap(),
                tag(&["parent", "p"]).unwrap(),
                tag(&["h-target", "c1"]).unwrap(),
                tag(&["h-target", "c2"]).unwrap(),
                tag(&["add", "bk", "r"]).unwrap(),
            ])
            .allow_self_tagging(),
    );
    assert!(parse_orchestration(&ev).is_none());
}

#[test]
fn parse_none_when_no_add_tags() {
    let ev = sign(
        EventBuilder::new(kind(KIND_CHAT), "x")
            .tags([
                tag(&["h", "p"]).unwrap(),
                tag(&["mosaico-op", MOSAICO_OP_ADD_AGENTS]).unwrap(),
                tag(&["parent", "p"]).unwrap(),
                tag(&["h-target", "c"]).unwrap(),
            ])
            .allow_self_tagging(),
    );
    assert!(parse_orchestration(&ev).is_none());
}

#[test]
fn parse_none_for_wrong_kind() {
    // A kind:1 with otherwise-valid tags is not an orchestration event.
    let ev = sign(
        EventBuilder::new(kind(1), "x")
            .tags([
                tag(&["h", "p"]).unwrap(),
                tag(&["mosaico-op", MOSAICO_OP_ADD_AGENTS]).unwrap(),
                tag(&["parent", "p"]).unwrap(),
                tag(&["h-target", "c"]).unwrap(),
                tag(&["add", "bk", "r"]).unwrap(),
            ])
            .allow_self_tagging(),
    );
    assert!(parse_orchestration(&ev).is_none());
}

#[test]
fn adds_for_backend_filters() {
    let adds = vec![
        at("bk1", "architect"),
        at("bk2", "engineer"),
        at("bk1", "qa"),
    ];
    let mine = adds_for_backend(&adds, "bk1");
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].slug, "architect");
    assert_eq!(mine[1].slug, "qa");

    assert!(adds_for_backend(&adds, "bk-none").is_empty());
}
