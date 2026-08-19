use super::*;
use crate::fabric::nip29::wire::{KIND_CHAT, KIND_GROUP_PUT_USER, KIND_REACTION, KIND_STATUS};

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

fn replace_ids(effects: &[SubEffect]) -> BTreeSet<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            SubEffect::Replace { id, .. } => Some(id.to_string()),
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
fn opens_one_broad_h_and_per_entity_and_is_idempotent() {
    let mut policy = SubscriptionReconciler::new();
    let snapshot = CoverageSnapshot {
        daemon_channels: set(["room-a", "room-b"]),
        profile_pubkeys: set(["backend-1"]),
        ..Default::default()
    };

    let effects = sync(&mut policy, &snapshot);
    // The N per-channel ChannelH observations and the global kind:9000
    // observation are gone — one broad #h covers all known channels (and 9000,
    // which carries #h). Only group-metadata global + per-author profiles stay.
    assert_eq!(
        open_ids(&effects),
        set([
            BROAD_H_ID,
            "mosaico-global-kind-39000",
            "mosaico-profile-backend-1",
        ])
    );
    assert_eq!(
        effects.iter().find_map(|effect| match effect {
            SubEffect::Open { id, query } if id == BROAD_H_ID => Some(query.clone()),
            _ => None,
        }),
        Some(SubscriptionQuery::GroupContentsMulti {
            groups: set(["room-a", "room-b"]),
            kinds: BTreeSet::from([KIND_CHAT, KIND_REACTION, KIND_STATUS, KIND_GROUP_PUT_USER]),
        })
    );
    assert_eq!(
        policy.applied_h_groups().cloned().unwrap(),
        set(["room-a", "room-b"])
    );
    assert!(sync(&mut policy, &snapshot).is_empty(), "idempotent");
}

/// A known-but-unjoined channel gets its relay-signed records hydrated without
/// its CONTENTS being subscribed to. The broad `#h` covers only daemon +
/// session channels, so a `group_state_channels` entry with no daemon/session
/// presence opens no content observation.
#[test]
fn known_unjoined_channels_do_not_subscribe_to_content() {
    let effects = SubscriptionReconciler::new().plan(&CoverageSnapshot {
        group_state_channels: set(["known-child"]),
        ..Default::default()
    });
    let opened = open_ids(&effects);

    assert!(opened.contains("mosaico-global-kind-39000"));
    assert!(!opened.contains(BROAD_H_ID));
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
    assert_eq!(
        query,
        &SubscriptionQuery::Profile {
            pubkey: "backend-1".to_string()
        }
    );
}

/// Under the broad `#h`, joining/leaving a channel does not open/close a
/// per-channel observation — it REPLACES the one broad observation with the new
/// channel set, and only when the set actually changes.
#[test]
fn broad_h_replaces_on_set_change_not_per_channel() {
    let mut policy = SubscriptionReconciler::new();
    sync(
        &mut policy,
        &CoverageSnapshot {
            sessions: sessions([("s1", set(["shared", "solo"])), ("s2", set(["shared"]))]),
            ..Default::default()
        },
    );
    assert_eq!(
        policy.applied_h_groups().cloned().unwrap(),
        set(["shared", "solo"])
    );

    // s1 leaves "shared" but s2 still has it → the union is unchanged.
    let first_leave = sync(
        &mut policy,
        &CoverageSnapshot {
            sessions: sessions([("s1", set(["solo"])), ("s2", set(["shared"]))]),
            ..Default::default()
        },
    );
    assert!(
        !replace_ids(&first_leave).contains(BROAD_H_ID),
        "no replace when the channel set is unchanged"
    );
    assert!(policy.covers_channel("shared"));

    // s2 leaves "shared" → the union loses it → broad #h is replaced.
    let last_leave = sync(
        &mut policy,
        &CoverageSnapshot {
            sessions: sessions([("s1", set(["solo"])), ("s2", BTreeSet::new())]),
            ..Default::default()
        },
    );
    assert!(
        replace_ids(&last_leave).contains(BROAD_H_ID),
        "replace when the channel set changes"
    );
    assert!(policy.covers_channel("solo"));
    assert!(!policy.covers_channel("shared"));
}

#[test]
fn daemon_channel_survives_session_leave_and_archived_never_in_broad_h() {
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
    assert!(
        !open_ids(&effects).iter().any(|id| id.contains("archived")),
        "archived channels never open"
    );
    assert_eq!(
        policy.applied_h_groups().cloned().unwrap(),
        set(["managed"]),
        "archived is excluded from the broad #h set"
    );

    // Session leaves, but "managed" is a daemon channel → stays in the set.
    let effects = sync(
        &mut policy,
        &CoverageSnapshot {
            daemon_channels: set(["managed"]),
            ..Default::default()
        },
    );
    assert!(
        !close_ids(&effects).contains(BROAD_H_ID),
        "no close while the daemon channel remains"
    );
    assert!(policy.covers_channel("managed"));
}

#[test]
fn broad_h_closes_when_no_channels_remain() {
    let mut policy = SubscriptionReconciler::new();
    sync(
        &mut policy,
        &CoverageSnapshot {
            daemon_channels: set(["room"]),
            ..Default::default()
        },
    );
    assert!(policy.applied_h_groups().is_some());

    let effects = sync(
        &mut policy,
        &CoverageSnapshot {
            ..Default::default()
        },
    );
    assert!(
        close_ids(&effects).contains(BROAD_H_ID),
        "broad #h closes when no channels remain"
    );
    assert!(policy.applied_h_groups().is_none());
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
    // Not confirmed → the next plan re-emits the same opens.
    let second = policy.plan(&snapshot);
    assert_eq!(open_ids(&first), open_ids(&second));
}
