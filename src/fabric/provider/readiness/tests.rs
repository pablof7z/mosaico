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
fn nip29_snapshot_proves_existing_member_ready() {
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
fn nip29_snapshot_does_not_prove_missing_member_ready() {
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
fn nip29_snapshot_does_not_prove_missing_admin_ready() {
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
fn managed_nip29_snapshot_is_not_ready_while_an_obsolete_admin_remains() {
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
fn nip29_subgroup_needs_relay_parent_consent_check() {
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
        "Nostr runtime root metadata must suppress a stale pending ancestor"
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
        "Nostr runtime root truth arriving before execution must suppress the captured hint"
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

/// A grant computed from a stale view can be refused by the relay as
/// ineffective. The relay's own group records, read fresh, decide: a target
/// that already holds the role is success; anything else stays a failure.
#[tokio::test]
async fn a_refused_role_grant_is_success_only_when_the_relay_holds_it() {
    use crate::fabric::group_management::{
        GroupMutationOutcome, GroupOperationError, GroupOperationStage,
    };
    use crate::nostr_runtime::tests::relay::{AckMode, TestRelay};
    use nostr::prelude::{EventBuilder, FinalizeEvent, Kind, Tag};

    let relay_keys = nostr::key::Keys::generate();
    let admin = nostr::key::Keys::generate().public_key();
    let stranger = nostr::key::Keys::generate().public_key();
    let admins_record = EventBuilder::new(Kind::Custom(39_001), "")
        .tags([
            Tag::custom("d", ["room"]),
            Tag::custom("p", [admin.to_hex(), "admin".to_string()]),
        ])
        .finalize(&relay_keys)
        .unwrap();
    let relay = TestRelay::new(AckMode::Accept, [admins_record]);
    let runtime =
        crate::nostr_runtime::tests::runtime(&relay, std::time::Duration::from_millis(500)).await;
    let provider = Nip29Provider::new(
        runtime,
        std::sync::Arc::new(crate::fabric::nip29::view::Nip29ViewStore::default()),
        std::sync::Arc::new(std::sync::Mutex::new(
            crate::state::Store::open_memory().unwrap(),
        )),
        nostr::key::Keys::generate().public_key(),
        None,
        Vec::new(),
    );
    let refused = || {
        GroupMutationOutcome::Failed(GroupOperationError::new(
            "9000 put-users (admins)",
            GroupOperationStage::Publish,
            "relay rejected event: blocked: all targets are members already",
        ))
    };

    admins::confirmed_or_published(
        &provider,
        "room",
        &[admin.to_hex()],
        "admin",
        refused(),
        "grant".into(),
    )
    .await
    .expect("a grant the relay already holds is confirmed");
    assert!(admins::confirmed_or_published(
        &provider,
        "room",
        &[stranger.to_hex()],
        "admin",
        refused(),
        "grant".into(),
    )
    .await
    .is_err());
}
