use super::{
    absolute::describe_missing_channel, absolute::resolve_absolute_channel_ref,
    absolute::root_channel, absolute::root_channel_by_slug, channel_reference_for, resolve_locally,
    ChannelResolution,
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
fn absolute_path_resolves_within_its_workspace() {
    let store = Store::open_memory().unwrap();
    chan(&store, "workspace", "general", "");
    chan(&store, "h-plan", "planning", "workspace");
    match resolve_absolute_channel_ref(&store, "/workspace/planning") {
        ChannelResolution::Unique(id) => assert_eq!(id, "h-plan"),
        _ => panic!("expected unique match"),
    }
}

#[test]
fn bare_relative_names_no_longer_resolve() {
    let store = Store::open_memory().unwrap();
    chan(&store, "workspace", "general", "");
    chan(&store, "h-plan", "planning", "workspace");
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
    chan(&store, "workspace", "general", "");
    chan(&store, "h-plan", "planning", "workspace");
    chan(&store, "h-epic", "epic999", "workspace");
    chan(&store, "h-epic-plan", "planning", "h-epic");

    // A full path names exactly one channel: no ambiguity, no suffix-matching
    // a deeper "planning" elsewhere in the tree.
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/workspace/planning"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/workspace/epic999/planning"),
        ChannelResolution::Unique(ref id) if id == "h-epic-plan"
    ));
}

#[test]
fn dots_are_not_aliases() {
    let store = Store::open_memory().unwrap();
    chan(&store, "workspace", "general", "");
    chan(&store, "h-epic", "epic", "workspace");
    chan(&store, "h-plan", "planning", "h-epic");
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/workspace/epic.planning"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn workspace_itself_resolves_by_slug() {
    let store = Store::open_memory().unwrap();
    chan(&store, "workspace", "general", "");
    chan(&store, "h-plan", "planning", "workspace");

    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/workspace"),
        ChannelResolution::Unique(ref id) if id == "workspace"
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/workspace/general"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn resolution_is_global_across_workspaces() {
    let store = Store::open_memory().unwrap();
    chan(&store, "nmp", "nmp", "");
    chan(&store, "other", "other", "");
    chan(&store, "h-child", "qa", "other");

    // A session "belonging" to /nmp can still resolve a path into /other
    // directly — there is no caller-scoped workspace restriction.
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/other/qa"),
        ChannelResolution::Unique(ref id) if id == "h-child"
    ));
    assert_eq!(
        root_channel_by_slug(&store, "other").as_deref(),
        Some("other")
    );
    assert_eq!(root_channel_by_slug(&store, "nonexistent"), None);
}

#[test]
fn channel_reference_prefers_unique_relative_path() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-epic", "epic", "h-root");
    chan(&store, "h-plan", "planning", "h-epic");

    assert_eq!(
        channel_reference_for(&store, "h-plan").unwrap(),
        "/h-root/epic/planning"
    );
}

#[test]
fn channel_reference_hides_internal_ids_when_ancestry_is_invalid() {
    let store = Store::open_memory().unwrap();
    chan(&store, "opaque-a", "a", "opaque-b");
    chan(&store, "opaque-b", "b", "opaque-a");

    let error = channel_reference_for(&store, "opaque-a")
        .unwrap_err()
        .to_string();
    assert_eq!(error, "channel has no complete agent-facing path");
    assert!(!error.contains("opaque-a"));
    assert!(!error.contains("opaque-b"));
}

#[test]
fn literal_id_selectors_are_rejected_in_every_form() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "proj", "");
    chan(&store, "h-plan", "planning", "h-root");
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "h-plan"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "@h-plan"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "@nonexistent"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/h-root/planning"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
}

#[test]
fn unnamed_internal_ancestry_cannot_be_bypassed_with_an_id_selector() {
    let store = Store::open_memory().unwrap();
    chan(&store, "h-root", "workspace", "");
    chan(&store, "session-room", "session-room", "h-root");
    chan(&store, "abcd1234", "editable", "session-room");

    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/h-root/editable"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "@abcd"),
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

    let root = root_channel(&store, "h-leaf").unwrap();
    assert_eq!(root, "h-root");
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "/h-root/epic/review"),
        ChannelResolution::Unique(ref id) if id == "h-review"
    ));
}

#[test]
fn describe_missing_channel_lists_the_workspace_and_sibling_workspaces() {
    let store = Store::open_memory().unwrap();
    chan(&store, "workspace", "workspace", "");
    chan(&store, "h-alpha", "alpha", "workspace");
    chan(&store, "test", "test", "");
    chan(&store, "h-foo", "foo", "test");
    chan(&store, "hello", "hello", "");

    let message = describe_missing_channel(&store, "/workspace/test/hello");
    assert!(message.contains("no channel matching"));
    assert!(message.contains("Channels in /workspace:"));
    assert!(message.contains("/workspace/alpha"));
    assert!(message.contains("/test is also a separate workspace"));
    assert!(message.contains("/test/foo"));
    assert!(message.contains("/hello is also a separate workspace"));
}

#[test]
fn describe_missing_channel_lists_known_workspaces_when_root_missing() {
    let store = Store::open_memory().unwrap();
    chan(&store, "nmp", "general", "");
    let message = describe_missing_channel(&store, "/nonexistent/child");
    assert!(message.contains("No workspace named \"nonexistent\""));
    assert!(message.contains("/nmp"));
}
