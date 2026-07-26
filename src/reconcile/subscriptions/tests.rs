use super::*;

fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
    items.into_iter().map(str::to_string).collect()
}

fn sessions<const N: usize>(
    entries: [(&str, BTreeSet<String>); N],
) -> BTreeMap<String, BTreeSet<String>> {
    entries
        .into_iter()
        .map(|(id, channels)| (id.to_string(), channels))
        .collect()
}

fn open_ids(effects: &[SubEffect]) -> BTreeSet<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            SubEffect::Open { id, .. } => Some(id.to_string()),
            _ => None,
        })
        .collect()
}

fn close_ids(effects: &[SubEffect]) -> BTreeSet<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            SubEffect::Close { id } => Some(id.to_string()),
            _ => None,
        })
        .collect()
}

fn sync(policy: &mut SubscriptionReconciler, snapshot: &CoverageSnapshot) -> Vec<SubEffect> {
    let effects = policy.plan(snapshot);
    for effect in &effects {
        policy.confirm(effect);
    }
    effects
}

#[test]
fn opens_one_narrow_req_per_entity_and_is_idempotent() {
    let mut policy = SubscriptionReconciler::new();
    let snapshot = CoverageSnapshot {
        daemon_channels: set(["room-a", "room-b"]),
        addressed_pubkeys: set(["pk-1", "pk-2"]),
        profile_pubkeys: set(["backend-1"]),
        ..Default::default()
    };

    assert_eq!(
        open_ids(&sync(&mut policy, &snapshot)),
        set([
            "mosaico-global-kind-9000",
            "mosaico-global-kind-39000",
            "mosaico-h-room-a",
            "mosaico-h-room-b",
            "mosaico-gstate-room-a",
            "mosaico-gstate-room-b",
            "mosaico-p-pk-1",
            "mosaico-p-pk-2",
            "mosaico-profile-backend-1",
        ])
    );
    assert!(sync(&mut policy, &snapshot).is_empty());
}

#[test]
fn known_unjoined_channels_hydrate_group_state_without_subscribing_to_content() {
    let effects = SubscriptionReconciler::new().plan(&CoverageSnapshot {
        group_state_channels: set(["known-child"]),
        ..Default::default()
    });
    let opened = open_ids(&effects);

    assert!(opened.contains("mosaico-global-kind-39000"));
    assert!(opened.contains("mosaico-gstate-known-child"));
    assert!(!opened.contains("mosaico-h-known-child"));
}

#[test]
fn host_profiles_are_observed_by_exact_author() {
    let effects = SubscriptionReconciler::new().plan(&CoverageSnapshot {
        profile_pubkeys: set(["backend-1"]),
        ..Default::default()
    });
    let query = effects
        .iter()
        .find_map(|effect| match effect {
            SubEffect::Open { id, query } if id == "mosaico-profile-backend-1" => Some(query),
            _ => None,
        })
        .expect("profile observation");
    assert_eq!(query.kinds, BTreeSet::from([0]));
    assert_eq!(query.authors, set(["backend-1"]));
    assert!(query.tag.is_none());
}

#[test]
fn channel_closes_only_when_last_owner_leaves() {
    let mut policy = SubscriptionReconciler::new();
    sync(
        &mut policy,
        &CoverageSnapshot {
            sessions: sessions([("s1", set(["shared", "solo"])), ("s2", set(["shared"]))]),
            ..Default::default()
        },
    );
    assert_eq!(policy.owner_count(Space::ChannelH, "shared"), 2);

    let first_leave = sync(
        &mut policy,
        &CoverageSnapshot {
            sessions: sessions([("s1", set(["solo"])), ("s2", set(["shared"]))]),
            ..Default::default()
        },
    );
    assert!(!close_ids(&first_leave).contains("mosaico-h-shared"));
    assert_eq!(policy.owner_count(Space::ChannelH, "shared"), 1);

    let last_leave = sync(
        &mut policy,
        &CoverageSnapshot {
            sessions: sessions([("s1", set(["solo"])), ("s2", BTreeSet::new())]),
            ..Default::default()
        },
    );
    let closed = close_ids(&last_leave);
    assert!(closed.contains("mosaico-h-shared"));
    assert!(closed.contains("mosaico-gstate-shared"));
    assert!(policy.covers_channel("solo"));
    assert!(!policy.covers_channel("shared"));
}

#[test]
fn daemon_owner_survives_session_leave_and_archived_channels_never_open() {
    let mut policy = SubscriptionReconciler::new();
    let effects = sync(
        &mut policy,
        &CoverageSnapshot {
            daemon_channels: set(["managed", "archived"]),
            archived_channels: set(["archived"]),
            sessions: sessions([("s1", set(["managed", "archived"]))]),
            ..Default::default()
        },
    );
    assert!(!open_ids(&effects).iter().any(|id| id.contains("archived")));
    assert_eq!(policy.owner_count(Space::ChannelH, "managed"), 2);

    let effects = sync(
        &mut policy,
        &CoverageSnapshot {
            daemon_channels: set(["managed"]),
            ..Default::default()
        },
    );
    assert!(!close_ids(&effects).contains("mosaico-h-managed"));
    assert!(policy.covers_channel("managed"));
}

#[test]
fn failed_effect_is_planned_again() {
    let mut policy = SubscriptionReconciler::new();
    let snapshot = CoverageSnapshot {
        daemon_channels: set(["room"]),
        ..Default::default()
    };
    let first = policy.plan(&snapshot);
    assert!(!first.is_empty());
    let second = policy.plan(&snapshot);
    assert_eq!(open_ids(&first), open_ids(&second));
}
