use super::{
    channel_reference_for, resolve_channel_ref, resolve_locally, root_channel, ChannelResolution,
};
use crate::state::Store;

fn chan(store: &Store, id: &str, name: &str, parent: &str) {
    store.upsert_channel(id, name, "", parent, 1).unwrap();
}

/// An 8-hex opaque id absent from the local cache (a freshly provisioned
/// channel whose kind:39000 hasn't materialized yet) resolves to ITSELF and
/// does NOT mint a literal-named channel — the launch-channel-scope fix.
#[test]
fn opaque_id_miss_passes_through_without_minting() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    let id = "2f1cd36f";
    assert!(
        store.get_channel(id).unwrap().is_none(),
        "precondition: opaque id must be absent from the cache"
    );
    assert_eq!(
        resolve_locally(&store, "h-root", id).unwrap(),
        Some(id.to_string()),
        "an unknown opaque id must pass through unchanged, not be minted"
    );
    // Passthrough is pure: it must not have created a channel named after the id.
    assert!(
        store.get_channel(id).unwrap().is_none(),
        "resolve_locally must never mint a channel"
    );
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
        store.get_channel("tenex-edge").unwrap().is_none(),
        "precondition: root must be absent from the cold cache"
    );
    assert_eq!(
        resolve_locally(&store, "tenex-edge", "tenex-edge").unwrap(),
        Some("tenex-edge".to_string()),
        "name==parent (the root asking for itself) must resolve to the slug, not mint a child"
    );
    assert!(
        store.get_channel("tenex-edge").unwrap().is_none(),
        "resolve_locally must never mint a channel"
    );
}

/// Known names/ids resolve locally; a genuine human NAME with no row does NOT
/// (the caller mints/bails) — proving the opaque-id passthrough never
/// over-triggers on real handles.
#[test]
fn known_resolve_locally_but_unknown_human_name_does_not() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-plan", "planning", "h-root");
    // 1. existing (parent, name) row wins.
    assert_eq!(
        resolve_locally(&store, "h-root", "planning").unwrap(),
        Some("h-plan".to_string())
    );
    // 2. a literal known channel_h passes through.
    assert_eq!(
        resolve_locally(&store, "h-root", "h-plan").unwrap(),
        Some("h-plan".to_string())
    );
    // 3. a genuine human NAME with no local row is unresolved here.
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
                vec!["epic999/planning".to_string(), "planning".to_string()]
            );
        }
        _ => panic!("expected ambiguous"),
    }
    // A fuller path disambiguates.
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "epic999/planning"),
        ChannelResolution::Unique(ref id) if id == "h-epic-plan"
    ));
}

/// Both `/` and `.` delimit path segments, so a dotted reference resolves
/// identically to the slashed one (the hierarchical-path change).
#[test]
fn dotted_path_resolves_same_as_slashed() {
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
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
}

#[test]
fn same_level_name_collision_falls_back_to_id_escape_hatch() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    // Two siblings share the name "planning" — a path cannot disambiguate.
    chan(&store, "h-aaaa1111", "planning", "h-root");
    chan(&store, "h-bbbb2222", "planning", "h-root");
    match resolve_channel_ref(&store, "h-root", "planning") {
        ChannelResolution::Ambiguous(refs) => {
            assert_eq!(refs, vec!["@h-aaaa11".to_string(), "@h-bbbb22".to_string()]);
        }
        _ => panic!("expected ambiguous id-escape-hatch"),
    }
    // The @id escape hatch then resolves uniquely.
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "@h-aaaa1"),
        ChannelResolution::Unique(ref id) if id == "h-aaaa1111"
    ));
}

#[test]
fn channel_reference_prefers_unique_relative_path() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-epic", "epic", "h-root");
    chan(&store, "h-plan", "planning", "h-epic");

    assert_eq!(channel_reference_for(&store, "h-plan"), "epic/planning");
}

#[test]
fn channel_reference_falls_back_to_id_on_duplicate_path() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-epic-a", "epic", "h-root");
    chan(&store, "h-epic-b", "epic", "h-root");
    chan(&store, "h-plan-a111", "planning", "h-epic-a");
    chan(&store, "h-plan-b222", "planning", "h-epic-b");

    assert_eq!(channel_reference_for(&store, "h-plan-a111"), "@h-plan-a");
    assert_eq!(channel_reference_for(&store, "h-plan-b222"), "@h-plan-b");
}

#[test]
fn literal_id_passthrough_and_not_found() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-plan", "planning", "h-root");
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "h-plan"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
    assert!(matches!(
        resolve_channel_ref(&store, "h-root", "nonexistent"),
        ChannelResolution::NotFound
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

    let root = root_channel(&store, "h-leaf");
    assert_eq!(root, "h-root");
    assert!(matches!(
        resolve_channel_ref(&store, &root, "epic/review"),
        ChannelResolution::Unique(ref id) if id == "h-review"
    ));
}
