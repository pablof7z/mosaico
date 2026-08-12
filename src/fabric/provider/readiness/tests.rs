use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn ready_snapshot(
    parent: &str,
    admins: &[nostr::PublicKey],
    members: &[nostr::PublicKey],
) -> nmp::nip29::GroupSnapshot {
    use nmp::nip29::{GroupAvailability, GroupMetadata, HostRecords, ListedRecord, ListedSubject};
    let host = nostr::RelayUrl::parse("wss://relay.example").unwrap();
    let subjects = |keys: &[nostr::PublicKey]| {
        keys.iter()
            .map(|pubkey| ListedSubject {
                pubkey: *pubkey,
                role: None,
                hosts: BTreeSet::from([host.clone()]),
            })
            .collect::<Vec<_>>()
    };
    let admins = subjects(admins);
    let members = subjects(members);
    let record = |subjects| ListedRecord {
        subjects,
        as_of: nostr::Timestamp::from(101),
        event_id: nostr::EventId::all_zeros(),
        host: host.clone(),
    };
    let metadata = GroupMetadata {
        name: Some("room".into()),
        about: None,
        picture: None,
        tags: vec![vec!["parent".into(), parent.into()]],
        as_of: nostr::Timestamp::from(100),
        event_id: nostr::EventId::all_zeros(),
        host: host.clone(),
    };
    nmp::nip29::GroupSnapshot {
        id: "room".into(),
        metadata: Some(metadata.clone()),
        admins: admins.clone(),
        members: members.clone(),
        availability: GroupAvailability::Ready,
        per_host: BTreeMap::from([(
            host.clone(),
            HostRecords {
                metadata: Some(metadata),
                admins: Some(record(admins)),
                members: Some(record(members)),
                availability: GroupAvailability::Ready,
            },
        )]),
        disagreements: BTreeSet::new(),
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
fn nmp_snapshot_proves_existing_member_ready() {
    let admin = nostr::Keys::generate().public_key();
    let member = nostr::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[admin], &[member]);

    assert!(local::snapshot_ready(
        &snapshot,
        &context(&member.to_hex(), None),
        &[admin.to_hex()],
        false,
    ));
}

#[test]
fn nmp_snapshot_does_not_prove_missing_member_ready() {
    let admin = nostr::Keys::generate().public_key();
    let member = nostr::Keys::generate().public_key();
    let other = nostr::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[admin], &[member]);

    assert!(!local::snapshot_ready(
        &snapshot,
        &context(&other.to_hex(), None),
        &[admin.to_hex()],
        false,
    ));
}

#[test]
fn nmp_snapshot_does_not_prove_missing_admin_ready() {
    let admin = nostr::Keys::generate().public_key();
    let other_admin = nostr::Keys::generate().public_key();
    let member = nostr::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[other_admin], &[member]);

    assert!(!local::snapshot_ready(
        &snapshot,
        &context(&member.to_hex(), None),
        &[admin.to_hex()],
        false,
    ));
}

#[test]
fn managed_nmp_snapshot_is_not_ready_while_an_obsolete_admin_remains() {
    let admin = nostr::Keys::generate().public_key();
    let obsolete = nostr::Keys::generate().public_key();
    let member = nostr::Keys::generate().public_key();
    let snapshot = ready_snapshot("", &[admin, obsolete], &[member]);

    assert!(!local::snapshot_ready(
        &snapshot,
        &context(&member.to_hex(), None),
        &[admin.to_hex()],
        true,
    ));
}

#[test]
fn nmp_subgroup_needs_relay_parent_consent_check() {
    let admin = nostr::Keys::generate().public_key();
    let member = nostr::Keys::generate().public_key();
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
        "NMP root metadata must suppress a stale pending ancestor"
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
        "NMP root truth arriving before execution must suppress the captured hint"
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
async fn managed_admin_removal_returns_the_exact_nmp_failure_without_local_roster_patch() {
    use crate::state::{TestGroup, TestGroupDelivery};

    let configured = nostr::Keys::generate().public_key().to_hex();
    let removed = nostr::Keys::generate().public_key().to_hex();
    let state =
        crate::daemon::server::DaemonState::new_for_test_with_whitelisted(vec![configured.clone()])
            .await;
    let management = state.fabric_provider().management_pubkey().unwrap();
    let snapshot = ready_snapshot(
        "",
        &[
            nostr::PublicKey::from_hex(&management).unwrap(),
            nostr::PublicKey::from_hex(&configured).unwrap(),
            nostr::PublicKey::from_hex(&removed).unwrap(),
        ],
        &[],
    );
    for _ in 0..2 {
        state.nmp().script_group_snapshot(snapshot.clone());
    }
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(TestGroupDelivery::new([TestGroup::new("room")
                .metadata("Room", "", "", 1)
                .admins(vec![management, configured, removed.clone()])
                .members(Vec::new())]));
            store.upsert_workspace("room", "/tmp/managed-room", 1)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    state
        .nmp()
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
