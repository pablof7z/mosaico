use super::groups::GroupProjection;
use super::*;
use nostr::{EventBuilder, EventId, Keys, Kind, RelayUrl, Timestamp};
use std::collections::{BTreeMap, BTreeSet};

mod availability;

fn row(content: &str) -> Row {
    Row {
        event: EventBuilder::new(Kind::TextNote, content)
            .sign_with_keys(&Keys::generate())
            .unwrap(),
        sources: BTreeSet::new(),
    }
}

fn group(id: &str, availability: nmp::nip29::GroupAvailability) -> nmp::nip29::GroupSnapshot {
    nmp::nip29::GroupSnapshot {
        id: id.to_string(),
        metadata: None,
        admins: vec![],
        members: vec![],
        availability,
        per_host: BTreeMap::new(),
        disagreements: BTreeSet::new(),
    }
}

fn subject(keys: &Keys, role: &str, host: &RelayUrl) -> nmp::nip29::ListedSubject {
    nmp::nip29::ListedSubject {
        pubkey: keys.public_key(),
        role: Some(role.to_string()),
        hosts: BTreeSet::from([host.clone()]),
    }
}

fn metadata(
    host: &RelayUrl,
    name: &str,
    about: &str,
    parent: &str,
    as_of: u64,
) -> nmp::nip29::GroupMetadata {
    nmp::nip29::GroupMetadata {
        name: Some(name.to_string()),
        about: Some(about.to_string()),
        picture: None,
        tags: vec![vec!["parent".to_string(), parent.to_string()]],
        as_of: Timestamp::from(as_of),
        event_id: EventId::all_zeros(),
        host: host.clone(),
    }
}

#[test]
fn group_projection_uses_only_the_snapshot_slice_the_caller_supplies() {
    let first = [
        group("a", nmp::nip29::GroupAvailability::Ready),
        group("b", nmp::nip29::GroupAvailability::Ready),
    ];
    let latest = [group("b", nmp::nip29::GroupAvailability::CachedOnly)];

    assert_eq!(
        GroupProjection::new(&first).group_availability("a"),
        Some(nmp::nip29::GroupAvailability::Ready)
    );
    let projection = GroupProjection::new(&latest);
    assert_eq!(
        projection.group_availability("b"),
        Some(nmp::nip29::GroupAvailability::CachedOnly)
    );
    assert_eq!(projection.group_availability("a"), None);
}

#[test]
fn test_delivery_replaces_atomically_and_uses_the_common_projection() {
    let views = NmpViews::default();
    views.install_test_group_delivery(TestGroupDelivery::new([TestGroup::new("old")
        .metadata("Old", "", "", 1)
        .availability(nmp::nip29::GroupAvailability::Ready)]));
    assert_eq!(
        views.with_groups(|groups| groups.group_availability("old")),
        Some(nmp::nip29::GroupAvailability::Ready)
    );

    views.install_test_group_delivery(TestGroupDelivery::new([TestGroup::new("current")
        .metadata("Current", "", "", 2)
        .availability(nmp::nip29::GroupAvailability::CachedOnly)]));
    views.with_groups(|groups| {
        assert!(groups.get_channel("old").is_none());
        assert_eq!(
            groups.group_availability("current"),
            Some(nmp::nip29::GroupAvailability::CachedOnly)
        );
    });
}

#[test]
fn group_queries_use_the_nmp_aggregate_without_merging_host_records() {
    let aggregate_host = RelayUrl::parse("wss://aggregate.example").unwrap();
    let other_host = RelayUrl::parse("wss://other.example").unwrap();
    let admin = Keys::generate();
    let member = Keys::generate();
    let host_only = Keys::generate();
    let admin_subject = subject(&admin, "relay-admin-role", &aggregate_host);
    let member_subject = subject(&member, "relay-member-role", &aggregate_host);
    let host_only_subject = subject(&host_only, "admin", &other_host);
    let other_metadata = metadata(&other_host, "host-only", "host-only", "wrong", 99);
    let other_record = nmp::nip29::ListedRecord {
        subjects: vec![host_only_subject],
        as_of: Timestamp::from(99),
        event_id: EventId::all_zeros(),
        host: other_host.clone(),
    };
    let snapshot = nmp::nip29::GroupSnapshot {
        id: "child".to_string(),
        metadata: Some(metadata(
            &aggregate_host,
            "Channel name",
            "Channel about",
            "root",
            7,
        )),
        admins: vec![admin_subject.clone()],
        members: vec![admin_subject, member_subject],
        availability: nmp::nip29::GroupAvailability::CachedOnly,
        per_host: BTreeMap::from([(
            other_host,
            nmp::nip29::HostRecords {
                metadata: Some(other_metadata),
                admins: Some(other_record),
                members: None,
                availability: nmp::nip29::GroupAvailability::Ready,
            },
        )]),
        disagreements: BTreeSet::new(),
    };
    let snapshots = [snapshot];
    let projection = GroupProjection::new(&snapshots);

    assert_eq!(
        projection.get_channel("child"),
        Some(crate::state::Channel {
            channel_h: "child".to_string(),
            name: "Channel name".to_string(),
            about: "Channel about".to_string(),
            parent: "root".to_string(),
            created_at: 7,
            updated_at: 7,
        })
    );
    assert_eq!(
        projection.group_availability("child"),
        Some(nmp::nip29::GroupAvailability::CachedOnly)
    );
    assert_eq!(
        projection.list_channel_members("child"),
        [
            crate::state::ChannelMember {
                channel_h: "child".to_string(),
                pubkey: admin.public_key().to_hex(),
                role: "admin".to_string(),
            },
            crate::state::ChannelMember {
                channel_h: "child".to_string(),
                pubkey: member.public_key().to_hex(),
                role: "member".to_string(),
            },
        ]
    );
    assert!(projection.is_channel_admin("child", &admin.public_key().to_hex()));
    assert!(projection.is_channel_member("child", &member.public_key().to_hex()));
    assert!(!projection.is_channel_member("child", &host_only.public_key().to_hex()));
    assert_eq!(
        projection.list_channels_where_member(&admin.public_key().to_hex()),
        ["child"]
    );
}

#[test]
fn high_level_group_queries_preserve_channel_topology_and_product_roles() {
    let host = RelayUrl::parse("wss://relay.example").unwrap();
    let admin = Keys::generate();
    let member = Keys::generate();
    let mut root = group("root", nmp::nip29::GroupAvailability::Ready);
    root.metadata = Some(metadata(&host, "general", "", "", 1));
    root.admins = vec![subject(&admin, "owner", &host)];
    let mut child = group("opaque", nmp::nip29::GroupAvailability::Ready);
    child.metadata = Some(metadata(&host, "task", "", "root", 2));
    child.members = vec![subject(&member, "participant", &host)];
    let snapshots = [child, root];
    let projection = GroupProjection::new(&snapshots);

    assert_eq!(
        projection
            .list_channels()
            .into_iter()
            .map(|channel| channel.channel_h)
            .collect::<Vec<_>>(),
        ["opaque", "root"]
    );
    assert_eq!(
        projection.channel_id_for_name("root", "task").as_deref(),
        Some("opaque")
    );
    assert_eq!(projection.channel_parent("opaque").as_deref(), Some("root"));
    assert_eq!(
        projection.root_channel_of("opaque").unwrap().as_deref(),
        Some("root")
    );
    assert!(projection.is_root_channel("root").unwrap());
    assert!(projection.is_subchannel("opaque").unwrap());
    assert_eq!(projection.list_root_channels()[0].channel_h, "root");
    assert_eq!(
        projection.list_child_channels("root")[0].channel_h,
        "opaque"
    );
    assert_eq!(
        projection.list_channels_where_admin(&admin.public_key().to_hex()),
        ["root"]
    );
    assert_eq!(projection.count_channel_members("opaque"), 1);
}

#[test]
fn overlapping_observations_do_not_become_a_second_event_owner() {
    let views = NmpViews::default();
    let shared = row("shared");
    let id = shared.event.id;
    let entered_a = views.apply_frame("a", 1, vec![RowDelta::Added(shared.clone())], vec![]);
    assert_eq!(entered_a.entered.len(), 1);
    assert_eq!(entered_a.added.len(), 1);
    let entered_b = views.apply_frame("b", 1, vec![RowDelta::Added(shared)], vec![]);
    assert_eq!(entered_b.entered.len(), 1);
    assert!(entered_b.added.is_empty());

    let departed_a = views.close_observation("a", 1).removed;
    assert_eq!(departed_a.len(), 1);
    assert_eq!(departed_a[0].observation_id, "a");
    assert_eq!(departed_a[0].row.event.id, id);
    assert!(views.row(&id).is_some());
    let departed_b = views.close_observation("b", 1).removed;
    assert_eq!(departed_b.len(), 1);
    assert_eq!(departed_b[0].observation_id, "b");
    assert_eq!(departed_b[0].row.event.id, id);
    assert!(views.row(&id).is_none());
}

#[test]
fn stale_observation_generation_cannot_overwrite_the_replacement() {
    let views = NmpViews::default();
    let old = row("old");
    let current = row("current");
    views.apply_frame("feed", 1, vec![RowDelta::Added(old.clone())], vec![]);
    views.apply_frame("feed", 2, vec![RowDelta::Added(current.clone())], vec![]);

    let stale = views.apply_frame("feed", 1, vec![RowDelta::Added(old)], vec![]);
    assert!(stale.added.is_empty());
    assert!(views.row(&current.event.id).is_some());
    assert_eq!(views.rows().len(), 1);
}

#[test]
fn opening_a_replacement_makes_the_predecessors_close_inert_without_a_gap() {
    let views = NmpViews::default();
    let current = row("current");
    views.apply_frame("feed", 1, vec![RowDelta::Added(current.clone())], vec![]);

    views.begin_observation("feed", 2);
    assert!(views.close_observation("feed", 1).removed.is_empty());
    assert!(views.row(&current.event.id).is_some());
}

#[test]
fn a_replacement_that_ends_before_its_first_frame_retires_the_previous_rows() {
    let views = NmpViews::default();
    let previous = row("previous");
    let id = previous.event.id;
    views.apply_frame("feed", 1, vec![RowDelta::Added(previous)], vec![]);

    views.begin_observation("feed", 2);
    let transition = views.close_observation("feed", 2);

    assert_eq!(transition.removed.len(), 1);
    assert_eq!(transition.removed[0].row.event.id, id);
    assert!(views.row(&id).is_none());
}
