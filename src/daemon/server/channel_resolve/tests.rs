use super::{
    channel_reference_for, resolve_channel_ref, resolve_locally, root_channel, ChannelResolution,
};
use crate::state::Store;

fn chan(store: &Store, id: &str, name: &str, parent: &str) {
    store.upsert_channel(id, name, "", parent, 1).unwrap();
}

/// A bare `launch` (no --channel) scopes to the channel root by resolving
/// `name == parent == slug`. On a COLD cache (post-reset, root kind:39000 not yet
/// materialized) this must resolve to the root slug itself and mint NOTHING —
/// the name-vs-id double-create regression (a spurious opaque child under root).
#[test]
fn root_slug_resolves_to_itself_on_cold_cache_without_minting() {
    let store = Store::open_memory().unwrap();
    // Empty cache: the channel root's kind:39000 has not materialized.
    assert!(
        store.get_channel("mosaico").unwrap().is_none(),
        "precondition: root must be absent from the cold cache"
    );
    assert_eq!(
        resolve_locally(&store, "mosaico", "mosaico").unwrap(),
        Some("mosaico".to_string()),
        "name==parent (the root asking for itself) must resolve to the slug, not mint a child"
    );
    assert!(
        store.get_channel("mosaico").unwrap().is_none(),
        "resolve_locally must never mint a channel"
    );
}

/// Known names resolve locally; a genuine human name with no row does not.
#[test]
fn known_name_resolves_locally_but_unknown_name_does_not() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-plan", "planning", "h-root");
    // An existing (parent, name) row wins.
    assert_eq!(
        resolve_locally(&store, "h-root", "planning").unwrap(),
        Some("h-plan".to_string())
    );
    // A genuine human name with no local row is unresolved here.
    assert_eq!(
        resolve_locally(&store, "h-root", "backlog-work").unwrap(),
        None
    );
}

#[test]
fn unique_relative_name_resolves() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-plan", "planning", "h-root");
    match resolve_channel_ref(&store, "h-root", "planning") {
        ChannelResolution::Unique(id) => assert_eq!(id, "h-plan"),
        _ => panic!("expected unique match"),
    }
}

#[test]
fn ambiguous_name_lists_relative_paths() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-plan", "planning", "h-root");
    chan(&store, "h-epic", "epic999", "h-root");
    chan(&store, "h-epic-plan", "planning", "h-epic");
    match resolve_channel_ref(&store, "h-root", "planning") {
        ChannelResolution::Ambiguous(refs) => {
            assert_eq!(
                refs,
                vec![
                    "h-root.epic999.planning".to_string(),
                    "h-root.planning".to_string()
                ]
            );
        }
        _ => panic!("expected ambiguous"),
    }
    // A fuller path disambiguates.
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "epic999.planning"),
        ChannelResolution::Unique(ref id) if id == "h-epic-plan"
    ));
}

#[test]
fn dotted_paths_are_canonical_and_slashes_are_not_aliases() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-epic", "epic", "h-root");
    chan(&store, "h-plan", "planning", "h-epic");
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "epic.planning"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "epic/planning"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn canonical_workspace_references_resolve_from_workspace_root() {
    let store = Store::open_memory().unwrap();
    chan(&store, "workspace", "workspace", "");
    chan(&store, "h-plan", "planning", "workspace");

    assert!(matches!(
        resolve_channel_ref(&store, "workspace", "workspace"),
        ChannelResolution::Unique(ref id) if id == "workspace"
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "workspace", "workspace.planning"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "workspace", "planning"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "workspace", "workspace.general"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn channel_reference_prefers_unique_relative_path() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-epic", "epic", "h-root");
    chan(&store, "h-plan", "planning", "h-epic");

    assert_eq!(
        channel_reference_for(&store, "h-plan").unwrap(),
        "h-root.epic.planning"
    );
}

#[test]
fn literal_id_requires_the_canonical_at_prefix() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-plan", "planning", "h-root");
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "h-plan"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "@h-plan"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "nonexistent"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn id_selector_reaches_named_channel_below_unnamed_session_room() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "workspace", "");
    chan(&store, "session-room", "session-room", "h-root");
    chan(&store, "abcd1234", "editable", "session-room");

    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "editable"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "@abcd"),
        ChannelResolution::Unique(ref id) if id == "abcd1234"
    ));
}

#[test]
fn nested_sender_explicit_channel_refs_resolve_from_root_channel() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-epic", "epic", "h-root");
    chan(&store, "h-plan", "planning", "h-epic");
    chan(&store, "h-leaf", "leaf", "h-plan");
    chan(&store, "h-review", "review", "h-epic");

    let root = root_channel(&store, "h-leaf").unwrap();
    assert_eq!(root, "h-root");
    assert!(matches!(
        resolve_channel_ref(&store, &root, "epic.review"),
        ChannelResolution::Unique(ref id) if id == "h-review"
    ));
}
