use super::{
    absolute::describe_missing_channel, absolute::resolve_absolute_channel_ref,
    absolute::root_channel, absolute::root_channel_by_slug, channel_reference_for, resolve_locally,
    ChannelResolution,
};
use crate::state::{Store, TestGroup, TestGroupDelivery};

#[path = "tests/selectors.rs"]
mod selectors;

fn channels(store: &Store, groups: &[(&str, &str, &str)]) {
    store.install_test_nmp_group_delivery(TestGroupDelivery::new(
        groups
            .iter()
            .map(|(id, name, parent)| TestGroup::new(id).metadata(name, "", parent, 1)),
    ));
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
    channels(
        &store,
        &[("h-root", "proj", ""), ("h-plan", "planning", "h-root")],
    );
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
fn absolute_path_resolves_within_its_workspace() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("workspace", "general", ""),
            ("h-plan", "planning", "workspace"),
        ],
    );
    match resolve_absolute_channel_ref(&store, "#workspace/planning") {
        ChannelResolution::Unique(id) => assert_eq!(id, "h-plan"),
        _ => panic!("expected unique match"),
    }
}

#[test]
fn bare_relative_names_no_longer_resolve() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("workspace", "general", ""),
            ("h-plan", "planning", "workspace"),
        ],
    );
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "planning"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "workspace/planning"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn a_full_path_is_exact_not_a_fuzzy_suffix_match() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("workspace", "general", ""),
            ("h-plan", "planning", "workspace"),
            ("h-epic", "epic999", "workspace"),
            ("h-epic-plan", "planning", "h-epic"),
        ],
    );

    // A full path names exactly one channel: no ambiguity, no suffix-matching
    // a deeper "planning" elsewhere in the tree.
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#workspace/planning"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#workspace/epic999/planning"),
        ChannelResolution::Unique(ref id) if id == "h-epic-plan"
    ));
}

#[test]
fn dots_are_not_aliases() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("workspace", "general", ""),
            ("h-epic", "epic", "workspace"),
            ("h-plan", "planning", "h-epic"),
        ],
    );
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#workspace/epic.planning"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn workspace_itself_resolves_by_slug() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("workspace", "general", ""),
            ("h-plan", "planning", "workspace"),
        ],
    );

    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#workspace"),
        ChannelResolution::Unique(ref id) if id == "workspace"
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#workspace/general"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn resolution_is_global_across_workspaces() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("nmp", "nmp", ""),
            ("other", "other", ""),
            ("h-child", "qa", "other"),
        ],
    );

    // A session "belonging" to /nmp can still resolve a path into /other
    // directly — there is no caller-scoped workspace restriction.
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#other/qa"),
        ChannelResolution::Unique(ref id) if id == "h-child"
    ));
    assert_eq!(
        root_channel_by_slug(&store, "other").as_deref(),
        Some("other")
    );
    assert_eq!(root_channel_by_slug(&store, "nonexistent"), None);
    assert_eq!(root_channel_by_slug(&store, "OTHER"), None);
}

#[test]
fn channel_reference_prefers_unique_relative_path() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("h-root", "proj", ""),
            ("h-epic", "epic", "h-root"),
            ("h-plan", "planning", "h-epic"),
        ],
    );

    assert_eq!(
        channel_reference_for(&store, "h-plan").unwrap(),
        "#h-root/epic/planning"
    );
}

#[test]
fn channel_reference_hides_internal_ids_when_ancestry_is_invalid() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[("opaque-a", "a", "opaque-b"), ("opaque-b", "b", "opaque-a")],
    );

    let error = channel_reference_for(&store, "opaque-a")
        .unwrap_err()
        .to_string();
    assert_eq!(error, "channel has no complete agent-facing path");
    assert!(!error.contains("opaque-a"));
    assert!(!error.contains("opaque-b"));
}
