//! What the roster is allowed to say about a member, and what it withholds.
//!
//! Three rules under test. A `state` label is a lifecycle claim, so only a live
//! heartbeat earns one. A member we cannot name is withheld rather than rendered
//! as a truncated pubkey. A member with neither a heartbeat nor a word spoken is
//! withheld too — its bare existence on the roster is not awareness.

use crate::fabric_context::{
    capture_inputs, render_fabric_context, render_full_session_state, ViewInputs,
};
use crate::state::{RelayEvent, Status, Store, TestGroupDelivery, TestRelayDelivery};

use super::{
    input, profile, root_group_with_roster, seed_profiles, seed_store, session, task_group,
    OTHER_PK, SELF_PK,
};

const GHOST_PK: &str = "ghost-pubkey";

fn said(id: &str, pubkey: &str, channel: &str, at: u64) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: pubkey.into(),
        created_at: at,
        channel_h: channel.into(),
        d_tag: String::new(),
        content: "on it".into(),
        tags_json: "[]".into(),
    }
}

fn heartbeat(pubkey: &str, slug: &str, state_since: u64) -> Status {
    Status {
        pubkey: pubkey.into(),
        channel_h: "root".into(),
        slug: slug.into(),
        title: "Reviewing".into(),
        activity: String::new(),
        workspace: "root".into(),
        branch: String::new(),
        state: crate::session_state::SessionState::Idle,
        state_since,
        last_seen: state_since,
        updated_at: state_since,
        expiration: 2_000,
    }
}

fn captured(store: &Store, rec: &crate::state::Session) -> ViewInputs {
    capture_inputs(store, &input(Some(rec), "root", 0, 100, true)).unwrap()
}

/// Just the `<agent>`/`<human>` rows, so an assertion about who is on the roster
/// cannot be satisfied (or defeated) by a reference appearing in the chatter.
fn member_rows(xml: &str) -> Vec<&str> {
    xml.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<agent ") || line.starts_with("<human "))
        .collect()
}

/// A peer with no presence lease is not necessarily gone — it may simply never
/// publish one. What it said in the channel is enough to place it, so it renders
/// with a real `since` and, deliberately, no `state`.
#[test]
fn message_activity_surfaces_a_member_the_heartbeat_never_reported() {
    let store = seed_store();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([said("msg-1", OTHER_PK, "root", 40)]),
    );
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        xml.contains("<agent name=\"@reviewer\" since=\"1 min ago\" />"),
        "{xml}"
    );
    assert!(
        !xml.contains("name=\"@reviewer\" state="),
        "activity is not a lifecycle claim, so it must not invent a state: {xml}"
    );
}

/// When a heartbeat does exist it owns the row. Older chatter must not drag
/// `since` backwards: `since` answers "since when has it been in this state",
/// and a message is not a state change.
#[test]
fn a_live_heartbeat_owns_the_state_label_and_its_own_since() {
    let store = seed_store();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([heartbeat(OTHER_PK, "reviewer", 95)])
            .events([said("msg-1", OTHER_PK, "root", 40)]),
    );
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        xml.contains(
            "<agent name=\"@reviewer\" state=\"idle\" status=\"Reviewing\" since=\"just now\" />"
        ),
        "{xml}"
    );
    assert!(
        !xml.contains("@reviewer\" state=\"idle\" status=\"Reviewing\" since=\"1 min ago\""),
        "the 40s-old message must not override the 95s state transition: {xml}"
    );
}

/// A roster member whose `kind:0` never arrived can only be printed as a
/// truncated pubkey. That is noise, so it is withheld until a current profile
/// row arrives.
#[test]
fn an_unnameable_member_is_withheld_until_a_profile_row_arrives() {
    let store = seed_store();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        root_group_with_roster(&[SELF_PK, OTHER_PK, GHOST_PK], &[]),
        task_group(),
    ]));
    let ghost_message = said("msg-1", GHOST_PK, "root", 40);
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([ghost_message.clone()]),
    );
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        !xml.lines()
            .find(|line| line.contains("<channel name=\"#root\""))
            .unwrap_or_default()
            .contains(" agents="),
        "an unclassified roster identity must make the count unknown: {xml}"
    );
    assert!(
        !member_rows(&xml).iter().any(|row| row.contains("ghost")),
        "{xml}"
    );

    // Once the kind:0 lands, the same member renders by name.
    let mut profiles = seed_profiles();
    let mut ghost = profile(GHOST_PK, "ghost", "ghost", "ghost", "laptop", false);
    ghost.updated_at = 3;
    profiles.push(ghost);
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(profiles)
            .events([ghost_message]),
    );
    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        xml.contains("<agent name=\"@ghost\" since=\"1 min ago\" />"),
        "{xml}"
    );
}

/// A live status carries its own public slug, which is a usable name even before
/// the profile arrives. Such a member renders immediately.
#[test]
fn a_status_slug_names_a_member_whose_profile_has_not_arrived() {
    let store = seed_store();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        root_group_with_roster(&[SELF_PK, GHOST_PK], &[]),
        task_group(),
    ]));
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([heartbeat(GHOST_PK, "amber-ghost", 95)]),
    );
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(xml.contains("<agent name=\"@amber-ghost\""), "{xml}");
}

/// A profile row exists but carries no slug — a `kind:0` that resolved to
/// nothing useful. That is not a handle, and the member is treated as unnamed.
#[test]
fn a_blank_slug_does_not_count_as_a_handle() {
    let store = seed_store();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        root_group_with_roster(&[SELF_PK, GHOST_PK], &[]),
        task_group(),
    ]));
    let mut profiles = seed_profiles();
    let mut ghost = profile(GHOST_PK, "Ghost", "   ", "", "laptop", false);
    ghost.updated_at = 2;
    profiles.push(ghost);
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(profiles)
            .events([said("msg-1", GHOST_PK, "root", 40)]),
    );
    let rec = session(&store);

    let inputs = captured(&store, &rec);
    assert!(!inputs.members.has_handle(GHOST_PK));
}

/// The briefing RPC renders through the same path as a hook turn, so it also
/// withholds a member until a current profile row gives it a usable name.
#[test]
fn the_full_session_briefing_withholds_an_unnameable_member() {
    let store = seed_store();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        root_group_with_roster(&[SELF_PK, GHOST_PK], &[]),
        task_group(),
    ]));
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([said("msg-1", GHOST_PK, "root", 40)]),
    );
    let rec = session(&store);

    let fabric =
        render_full_session_state(&store, &rec, "coder", "", "laptop", 100).expect("briefing");
    assert!(
        !member_rows(&fabric).iter().any(|row| row.contains("ghost")),
        "{fabric}"
    );
}
