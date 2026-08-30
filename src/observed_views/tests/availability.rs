use super::*;
use crate::group_snapshot::GroupAvailability;

#[test]
fn group_availability_is_the_exact_fava_answer() {
    for availability in [
        GroupAvailability::Acquiring,
        GroupAvailability::Ready,
        GroupAvailability::CachedOnly,
    ] {
        let snapshots = [group("room", availability)];
        assert_eq!(
            GroupProjection::new(&snapshots).group_availability("room"),
            Some(availability)
        );
    }
}

#[test]
fn an_empty_settled_replacement_removes_the_previous_rows() {
    let views = ObservedViews::default();
    let previous = row("previous");
    let previous_id = previous.event.id;
    views.apply_frame("feed", 1, vec![RowDelta::Added(previous)]);

    let transition = views.apply_frame("feed", 2, Vec::new());

    assert_eq!(transition.removed.len(), 1);
    assert_eq!(transition.removed[0].row.event.id, previous_id);
    assert!(transition.added.is_empty());
    assert!(views.row(&previous_id).is_none());
}
