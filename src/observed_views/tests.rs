use super::groups::GroupProjection;
use super::*;
use crate::group_snapshot::{GroupAvailability, GroupMetadata, GroupSnapshot, ListedSubject};
use nostr::event::{EventBuilder, FinalizeEvent, Kind};
use nostr::key::Keys;

mod availability;

fn row(content: &str) -> Row {
    Row {
        event: EventBuilder::new(Kind::TextNote, content)
            .finalize(&Keys::generate())
            .unwrap(),
    }
}

fn group(id: &str, availability: GroupAvailability) -> GroupSnapshot {
    GroupSnapshot {
        id: id.to_string(),
        metadata: None,
        admins: vec![],
        members: vec![],
        availability,
    }
}

fn subject(keys: &Keys, role: &str) -> ListedSubject {
    ListedSubject::new(keys.public_key(), vec![role.to_string()])
}

fn metadata(name: &str, about: &str, parent: &str, as_of: u64) -> GroupMetadata {
    GroupMetadata {
        name: Some(name.to_string()),
        about: Some(about.to_string()),
        tags: vec![vec!["parent".to_string(), parent.to_string()]],
        as_of: std::time::Duration::from_secs(as_of),
    }
}

#[test]
fn group_projection_uses_only_the_snapshot_slice_the_caller_supplies() {
    let first = [
        group("a", GroupAvailability::Ready),
        group("b", GroupAvailability::Ready),
    ];
    let latest = [group("b", GroupAvailability::CachedOnly)];

    assert_eq!(
        GroupProjection::new(&first).group_availability("a"),
        Some(GroupAvailability::Ready)
    );
    let projection = GroupProjection::new(&latest);
    assert_eq!(
        projection.group_availability("b"),
        Some(GroupAvailability::CachedOnly)
    );
    assert_eq!(projection.group_availability("a"), None);
}

#[test]
fn test_delivery_replaces_atomically_and_uses_the_common_projection() {
    let views = ObservedViews::default();
    views.install_test_group_delivery(TestGroupDelivery::new([TestGroup::new("old")
        .metadata("Old", "", "", 1)
        .availability(GroupAvailability::Ready)]));
    assert_eq!(
        views.with_groups(|groups| groups.group_availability("old")),
        Some(GroupAvailability::Ready)
    );

    views.install_test_group_delivery(TestGroupDelivery::new([TestGroup::new("current")
        .metadata("Current", "", "", 2)
        .availability(GroupAvailability::CachedOnly)]));
    views.with_groups(|groups| {
        assert!(groups.get_channel("old").is_none());
        assert_eq!(
            groups.group_availability("current"),
            Some(GroupAvailability::CachedOnly)
        );
    });
}

#[test]
fn high_level_group_queries_preserve_channel_topology_and_product_roles() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let mut root = group("root", GroupAvailability::Ready);
    root.metadata = Some(metadata("general", "", "", 1));
    root.admins = vec![subject(&admin, "owner")];
    let mut child = group("opaque", GroupAvailability::Ready);
    child.metadata = Some(metadata("task", "", "root", 2));
    child.members = vec![subject(&member, "participant")];
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
    let views = ObservedViews::default();
    let shared = row("shared");
    let id = shared.event.id;
    let entered_a = views.apply_frame("a", 1, vec![RowDelta::Added(shared.clone())]);
    assert_eq!(entered_a.entered.len(), 1);
    assert_eq!(entered_a.added.len(), 1);
    let entered_b = views.apply_frame("b", 1, vec![RowDelta::Added(shared)]);
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
    let views = ObservedViews::default();
    let old = row("old");
    let current = row("current");
    views.apply_frame("feed", 1, vec![RowDelta::Added(old.clone())]);
    views.apply_frame("feed", 2, vec![RowDelta::Added(current.clone())]);

    let stale = views.apply_frame("feed", 1, vec![RowDelta::Added(old)]);
    assert!(stale.added.is_empty());
    assert!(views.row(&current.event.id).is_some());
    assert_eq!(views.rows().len(), 1);
}

#[test]
fn opening_a_replacement_makes_the_predecessors_close_inert_without_a_gap() {
    let views = ObservedViews::default();
    let current = row("current");
    views.apply_frame("feed", 1, vec![RowDelta::Added(current.clone())]);

    views.begin_observation("feed", 2);
    assert!(views.close_observation("feed", 1).removed.is_empty());
    assert!(views.row(&current.event.id).is_some());
}

#[test]
fn a_replacement_that_ends_before_its_first_frame_retires_the_previous_rows() {
    let views = ObservedViews::default();
    let previous = row("previous");
    let id = previous.event.id;
    views.apply_frame("feed", 1, vec![RowDelta::Added(previous)]);

    views.begin_observation("feed", 2);
    let transition = views.close_observation("feed", 2);

    assert_eq!(transition.removed.len(), 1);
    assert_eq!(transition.removed[0].row.event.id, id);
    assert!(views.row(&id).is_none());
}
