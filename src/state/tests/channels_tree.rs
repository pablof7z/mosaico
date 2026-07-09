use super::super::*;

#[test]
fn channels_root_vs_subchannel() {
    let s = Store::open_memory().unwrap();
    s.upsert_channel("proj", "P", "", "", 1).unwrap();
    s.upsert_channel("task", "T", "", "proj", 1).unwrap();
    assert!(s.is_root_channel("proj").unwrap());
    assert!(!s.is_root_channel("task").unwrap());
    assert_eq!(s.channel_parent("task").unwrap().unwrap(), "proj");
    assert_eq!(s.root_channel_of("task").unwrap().as_deref(), Some("proj"));
    assert!(s.is_subchannel("task").unwrap());
    assert!(!s.is_subchannel("proj").unwrap());
    assert_eq!(s.root_channel_of("missing").unwrap(), None);
    assert!(!s.is_root_channel("missing").unwrap());
    assert!(!s.is_subchannel("missing").unwrap());
}

#[test]
fn root_channel_of_walks_nested_tree_strictly() {
    let s = Store::open_memory().unwrap();
    s.upsert_channel("proj", "P", "", "", 1).unwrap();
    s.upsert_channel("epic", "Epic", "", "proj", 1).unwrap();
    s.upsert_channel("plan", "Plan", "", "epic", 1).unwrap();
    s.upsert_channel("leaf", "Leaf", "", "plan", 1).unwrap();

    assert_eq!(s.root_channel_of("leaf").unwrap().as_deref(), Some("proj"));
    assert!(!s.is_root_channel("leaf").unwrap());
    assert!(s.is_subchannel("leaf").unwrap());
    assert!(s.is_root_channel("proj").unwrap());
    assert!(!s.is_subchannel("proj").unwrap());
}

#[test]
fn root_channel_of_refuses_unknown_ancestor() {
    let s = Store::open_memory().unwrap();
    s.upsert_channel("leaf", "Leaf", "", "missing-parent", 1)
        .unwrap();

    assert_eq!(s.root_channel_of("leaf").unwrap(), None);
    assert!(!s.is_root_channel("leaf").unwrap());
    assert!(!s.is_subchannel("leaf").unwrap());
}

#[test]
fn channel_id_for_name_resolves_within_parent() {
    let s = Store::open_memory().unwrap();
    // Opaque id, human name "support" under channel "proj".
    s.upsert_channel("ab12cd34", "support", "", "proj", 10)
        .unwrap();
    assert_eq!(
        s.channel_id_for_name("proj", "support").unwrap().as_deref(),
        Some("ab12cd34")
    );
    // Unknown name → None.
    assert_eq!(s.channel_id_for_name("proj", "nope").unwrap(), None);
    // Same name under a DIFFERENT parent is a distinct channel (allowed).
    s.upsert_channel("ff99ff99", "support", "", "other", 10)
        .unwrap();
    assert_eq!(
        s.channel_id_for_name("other", "support")
            .unwrap()
            .as_deref(),
        Some("ff99ff99")
    );
    // Legacy duplicate (parent, name): most-recently-updated wins.
    s.upsert_channel("zz000000", "support", "", "proj", 20)
        .unwrap();
    assert_eq!(
        s.channel_id_for_name("proj", "support").unwrap().as_deref(),
        Some("zz000000")
    );
}

#[test]
fn channel_human_name_distinguishes_root_slug_from_unnamed_session_room() {
    let chan = |channel_h: &str, name: &str, parent: &str| Channel {
        channel_h: channel_h.into(),
        name: name.into(),
        about: String::new(),
        parent: parent.into(),
        created_at: 1,
        updated_at: 1,
    };
    assert_eq!(
        chan("tenex-edge", "tenex-edge", "").human_name(),
        Some("tenex-edge")
    );
    assert_eq!(
        chan("ab12cd34", "support", "proj").human_name(),
        Some("support")
    );
    assert_eq!(chan("session-x1", "session-x1", "proj").human_name(), None);
    assert_eq!(chan("", "", "").human_name(), None);
    assert_eq!(chan("ab12cd34", "   ", "proj").human_name(), None);
}
