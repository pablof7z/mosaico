use super::*;
use crate::group_snapshot::{GroupAvailability, GroupMetadata, GroupSnapshot, ListedSubject};

fn ready_snapshot(
    parent: &str,
    admins: &[nostr::key::PublicKey],
    members: &[nostr::key::PublicKey],
) -> GroupSnapshot {
    let subjects = |keys: &[nostr::key::PublicKey]| {
        keys.iter()
            .map(|pubkey| ListedSubject::new(*pubkey, Vec::new()))
            .collect::<Vec<_>>()
    };
    let mut tags = Vec::new();
    if !parent.is_empty() {
        tags.push(vec!["parent".into(), parent.into()]);
    }
    GroupSnapshot {
        id: "room".into(),
        metadata: Some(GroupMetadata {
            name: Some("room".into()),
            about: None,
            tags,
            as_of: std::time::Duration::from_secs(100),
        }),
        admins: subjects(admins),
        members: subjects(members),
        availability: GroupAvailability::Ready,
    }
}

fn context<'a>(expect_member: &'a str, parent_hint: Option<&'a str>) -> ChannelCtx<'a> {
    ChannelCtx {
        channel: "room",
        expect_member,
        parent_hint,
        name: None,
    }
}

#[test]
fn fava_snapshot_proves_existing_member_ready() {
    let admin = nostr::key::Keys::generate().public_key();
    let member = nostr::key::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[admin], &[member]);

    assert!(local::snapshot_ready(
        &snapshot,
        &context(&member.to_hex(), None),
        &[admin.to_hex()],
        false,
    ));
}

#[test]
fn fava_snapshot_does_not_prove_missing_member_ready() {
    let admin = nostr::key::Keys::generate().public_key();
    let member = nostr::key::Keys::generate().public_key();
    let other = nostr::key::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[admin], &[member]);

    assert!(!local::snapshot_ready(
        &snapshot,
        &context(&other.to_hex(), None),
        &[admin.to_hex()],
        false,
    ));
}

#[test]
fn fava_snapshot_does_not_prove_missing_admin_ready() {
    let admin = nostr::key::Keys::generate().public_key();
    let other_admin = nostr::key::Keys::generate().public_key();
    let member = nostr::key::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[other_admin], &[member]);

    assert!(!local::snapshot_ready(
        &snapshot,
        &context(&member.to_hex(), None),
        &[admin.to_hex()],
        false,
    ));
}

#[test]
fn managed_fava_snapshot_is_not_ready_while_an_obsolete_admin_remains() {
    let admin = nostr::key::Keys::generate().public_key();
    let obsolete = nostr::key::Keys::generate().public_key();
    let member = nostr::key::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[admin, obsolete], &[member]);

    assert!(!local::snapshot_ready(
        &snapshot,
        &context(&member.to_hex(), None),
        &[admin.to_hex()],
        true,
    ));
}

#[test]
fn fava_subgroup_needs_relay_parent_consent_check() {
    let admin = nostr::key::Keys::generate().public_key();
    let member = nostr::key::Keys::generate().public_key();
    let snapshot = ready_snapshot("parent", &[admin], &[member]);

    assert!(!local::snapshot_ready(
        &snapshot,
        &context(&member.to_hex(), Some("parent")),
        &[admin.to_hex()],
        false,
    ));
}

#[test]
fn cold_nested_resolution_intents_preserve_every_ancestor() {
    let store = crate::state::Store::open_memory().unwrap();
    store
        .reserve_channel_resolution_intent("root", "middle", "middle-h", 1)
        .unwrap();
    store
        .reserve_channel_resolution_intent("middle-h", "leaf", "leaf-h", 2)
        .unwrap();

    assert_eq!(
        ancestry::resolved_parent_hint_with_observed_parent(&store, "leaf-h", None, None).unwrap(),
        Some("middle-h".into())
    );
    assert_eq!(
        ancestry::resolved_parent_hint_with_observed_parent(&store, "middle-h", None, None)
            .unwrap(),
        Some("root".into())
    );
    assert_eq!(
        ancestry::resolved_parent_hint_with_observed_parent(&store, "root", None, None).unwrap(),
        None
    );

    assert_eq!(
        ancestry::resolved_parent_hint_with_observed_parent(
            &store,
            "middle-h",
            None,
            Some(String::new()),
        )
        .unwrap(),
        None,
        "Fava root metadata must suppress a stale pending ancestor"
    );
}

#[test]
fn execution_time_relay_metadata_overrides_captured_parent_hint() {
    let store = crate::state::Store::open_memory().unwrap();
    assert_eq!(
        ancestry::resolved_parent_hint_with_observed_parent(
            &store,
            "room",
            Some("captured-parent"),
            None,
        )
        .unwrap(),
        Some("captured-parent".into())
    );

    assert_eq!(
        ancestry::resolved_parent_hint_with_observed_parent(
            &store,
            "room",
            Some("captured-parent"),
            Some(String::new()),
        )
        .unwrap(),
        None,
        "Fava root truth arriving before execution must suppress the captured hint"
    );

    assert_eq!(
        ancestry::resolved_parent_hint_with_observed_parent(
            &store,
            "room",
            Some("captured-parent"),
            Some("relay-parent".into()),
        )
        .unwrap(),
        Some("relay-parent".into())
    );
}

#[tokio::test]
async fn managed_admin_removal_returns_the_exact_fava_failure_without_local_roster_patch() {
    use crate::state::{TestGroup, TestGroupDelivery};

    let configured = nostr::key::Keys::generate().public_key().to_hex();
    let removed = nostr::key::Keys::generate().public_key().to_hex();
    let state =
        crate::daemon::server::DaemonState::new_for_test_with_whitelisted(vec![configured.clone()])
            .await;
    let management = state.fabric_provider().management_pubkey().unwrap();
    let snapshot = ready_snapshot(
        "",
        &[
            nostr::key::PublicKey::from_hex(&management).unwrap(),
            nostr::key::PublicKey::from_hex(&configured).unwrap(),
            nostr::key::PublicKey::from_hex(&removed).unwrap(),
        ],
        &[],
    );
    for _ in 0..2 {
        state.fava().script_group_snapshot(snapshot.clone());
    }
    state
        .with_store(|store| {
            store.install_test_fava_group_delivery(TestGroupDelivery::new([TestGroup::new("room")
                .metadata("Room", "", "", 1)
                .admins(vec![management, configured, removed.clone()])
                .members(Vec::new())]));
            store.upsert_workspace("room", "/tmp/managed-room", 1)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    state
        .fava()
        .script_write_error("terminal receipt", "relay explicitly rejected removal");

    let error = state
        .fabric_provider()
        .reconcile_managed_admins("room", Some(&[]))
        .await
        .unwrap_err();
    let rendered = format!("{:#}", anyhow::Error::new(error));
    assert!(rendered.contains("one obsolete admin removal for 1 users"));
    assert!(rendered.contains("relay explicitly rejected removal"));
    assert!(state.with_store(|store| store.is_channel_admin("room", &removed).unwrap()));
}
