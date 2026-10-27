use std::collections::BTreeSet;

use nostr::prelude::{Event, EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use std::sync::Arc;

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
    assert_eq!(view.current().content().len(), 2);
    view.replace(vec![beta.clone()]);

    assert_eq!(view.current().content(), [beta]);
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
        view.current()
            .group_snapshots()
            .iter()
            .map(|group| group.id.clone())
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

    let current = view.current();
    let group = current.group_snapshots().first().unwrap();
    assert_eq!(group.id, "alpha");
    assert_eq!(
        group.metadata.as_ref().unwrap().name.as_deref(),
        Some("Alpha")
    );
    assert_eq!(group.admins[0].pubkey, admin.public_key());
    assert_eq!(group.admins[0].roles, ["admin"]);
    assert_eq!(group.members[0].pubkey, member.public_key());
}

#[test]
fn reads_share_one_level_until_the_next_replacement() {
    let view = Nip29ViewStore::default();
    let relay = Keys::generate();
    let member = Keys::generate();
    let record = |kind, extra: Vec<Tag>| {
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(std::iter::once(Tag::custom("d", ["alpha"])).chain(extra))
            .finalize(&relay)
            .unwrap()
    };
    view.replace_group_scope(BTreeSet::from(["alpha".to_string()]), BTreeSet::new());
    view.replace(vec![
        record(39_000, vec![Tag::custom("name", ["Alpha"])]),
        record(
            39_002,
            vec![Tag::custom("p", [member.public_key().to_hex()])],
        ),
    ]);

    let first = view.current();
    assert!(Arc::ptr_eq(&first, &view.current()));
    let members = first.groups().list_channel_members("alpha");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].pubkey, member.public_key().to_hex());

    view.replace(Vec::new());
    assert!(!Arc::ptr_eq(&first, &view.current()));
    assert!(view.current().groups().list_channels().is_empty());
}

#[test]
fn decode_derives_every_read_model_from_signed_events() {
    use crate::domain::{AgentRef, DomainEvent, Profile, Status};
    use crate::fabric::nip29::wire::Nip29WireCodec;

    let view = Nip29ViewStore::default();
    let agent = Keys::generate();
    let pubkey = agent.public_key().to_hex();
    let signed = |event: &DomainEvent| {
        Nip29WireCodec
            .encode_event(event)
            .unwrap()
            .finalize(&agent)
            .unwrap()
    };
    let profile = signed(&DomainEvent::Profile(Profile {
        agent: AgentRef::new(pubkey.clone(), "reviewer"),
        agent_slug: "reviewer".into(),
        host: "laptop".into(),
        workspace: "alpha".into(),
        owners: Vec::new(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
    }));
    // A status is a several-group write: the provider adds one `h` per channel.
    let status_builder = Nip29WireCodec
        .encode_event(&DomainEvent::Status(Status {
        agent: AgentRef::new(pubkey.clone(), ""),
        channels: vec!["alpha".into()],
        host: "laptop".into(),
        workspace: "alpha".into(),
        branch: String::new(),
        title: "reviewing".into(),
        activity: String::new(),
        state: crate::session_state::SessionState::Working,
        state_since: 1,
        rel_cwd: String::new(),
        expires_at: None,
        dispatch_event: None,
    }))
        .unwrap();
    let status =
        crate::fabric::nip29::signed_into_groups(&["alpha"], status_builder, &agent).unwrap();
    let chat = EventBuilder::new(Kind::Custom(9), "hello")
        .tags([
            Tag::custom("h", ["alpha"]),
            Tag::custom("p", [pubkey.as_str()]),
            Tag::custom("e", ["parent-id", "", "reply"]),
        ])
        .finalize(&agent)
        .unwrap();
    let reaction = EventBuilder::new(Kind::Custom(7), "👍")
        .tags([
            Tag::custom("h", ["alpha"]),
            Tag::custom("e", [chat.id.to_hex()]),
        ])
        .finalize(&agent)
        .unwrap();
    view.replace(vec![profile, status, chat.clone(), reaction]);

    let current = view.current();
    assert_eq!(current.profile(&pubkey).unwrap().slug, "reviewer");
    let statuses = current.statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].channel_h, "alpha");
    assert_eq!(statuses[0].slug, "reviewer", "slug enriched from the profile");
    let chat_id = chat.id.to_hex();
    assert_eq!(current.event(&chat_id).unwrap().channel_h, "alpha");
    let message = current.message(&chat_id).unwrap();
    assert_eq!(message.message.body, "hello");
    assert_eq!(message.reply_target.as_deref(), Some("parent-id"));
    assert_eq!(message.recipients[0].recipient_pubkey, pubkey);
    assert_eq!(current.reactions().len(), 1);
    assert_eq!(current.reactions()[0].target_message_id, chat_id);
}
