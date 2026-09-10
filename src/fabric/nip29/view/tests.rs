use std::collections::BTreeSet;

use nostr::prelude::{Event, EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::Nip29ViewStore;

fn event(keys: &Keys, group: &str, body: &str) -> Event {
    EventBuilder::new(Kind::Custom(9), body)
        .tag(Tag::custom("h", [group]))
        .finalize(keys)
        .unwrap()
}

#[test]
fn complete_replacement_retracts_demand_excluded_content() {
    let view = Nip29ViewStore::default();
    let keys = Keys::generate();
    let alpha = event(&keys, "alpha", "a");
    let beta = event(&keys, "beta", "b");

    view.replace(vec![alpha.clone(), beta.clone()]);
    assert_eq!(view.content_events().len(), 2);
    view.replace(vec![beta.clone()]);

    assert_eq!(view.content_events(), vec![beta]);
}

#[test]
fn group_projection_obeys_exact_current_scope() {
    let view = Nip29ViewStore::default();
    let relay = Keys::generate();
    let metadata = |group: &str| {
        EventBuilder::new(Kind::Custom(39_000), "")
            .tags([Tag::custom("d", [group]), Tag::custom("name", [group])])
            .finalize(&relay)
            .unwrap()
    };
    view.replace_group_scope(BTreeSet::from(["alpha".to_string()]), BTreeSet::new());
    view.replace(vec![metadata("alpha"), metadata("beta")]);

    assert_eq!(
        view.group_snapshots()
            .into_iter()
            .map(|group| group.id)
            .collect::<Vec<_>>(),
        vec!["alpha"]
    );
}

#[test]
fn relay_authored_group_records_project_metadata_admins_and_members() {
    let view = Nip29ViewStore::default();
    let relay = Keys::generate();
    let admin = Keys::generate();
    let member = Keys::generate();
    let record = |kind, extra: Vec<Tag>| {
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(std::iter::once(Tag::custom("d", ["alpha"])).chain(extra))
            .finalize(&relay)
            .unwrap()
    };
    view.replace_group_scope(BTreeSet::from(["alpha".to_string()]), BTreeSet::new());
    view.replace(vec![
        record(
            39_000,
            vec![
                Tag::custom("name", ["Alpha"]),
                Tag::custom("about", ["Coordination"]),
            ],
        ),
        record(
            39_001,
            vec![Tag::custom(
                "p",
                [admin.public_key().to_hex(), "admin".to_string()],
            )],
        ),
        record(
            39_002,
            vec![Tag::custom("p", [member.public_key().to_hex()])],
        ),
    ]);

    let groups = view.group_snapshots();
    let group = groups.first().unwrap();
    assert_eq!(group.id, "alpha");
    assert_eq!(
        group.metadata.as_ref().unwrap().name.as_deref(),
        Some("Alpha")
    );
    assert_eq!(group.admins[0].pubkey, admin.public_key());
    assert_eq!(group.admins[0].roles, ["admin"]);
    assert_eq!(group.members[0].pubkey, member.public_key());
}
