//! What the roster is allowed to say about a member, and what it withholds.
//!
//! Three rules under test. A `state` label is a lifecycle claim, so only a live
//! heartbeat earns one. A member we cannot name is withheld rather than rendered
//! as a truncated pubkey, and reported so its `kind:0` can be fetched. A member
//! with neither a heartbeat nor a word spoken is withheld too — its bare
//! existence on the roster is not awareness.

use crate::fabric_context::{
    capture_inputs, missing_profile_pubkeys, render_fabric_context, render_full_session_state,
    ViewInputs,
};
use crate::state::{RelayEvent, Status, Store};

use super::{input, seed_store, session, OTHER_PK, SELF_PK, TASK_H};

const GHOST_PK: &str = "ghost-pubkey";

fn say(store: &Store, id: &str, pubkey: &str, channel: &str, at: u64) {
    store
        .insert_event(&RelayEvent {
            id: id.into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: pubkey.into(),
            created_at: at,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: "on it".into(),
            tags_json: "[]".into(),
        })
        .unwrap();
}

fn heartbeat(store: &Store, pubkey: &str, slug: &str, state_since: u64) {
    store
        .upsert_status(&Status {
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
        })
        .unwrap();
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
    say(&store, "msg-1", OTHER_PK, "root", 40);
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
    say(&store, "msg-1", OTHER_PK, "root", 40);
    heartbeat(&store, OTHER_PK, "reviewer", 95);
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
/// truncated pubkey. That is noise, so it is withheld — and reported, so the
/// daemon can go fetch the profile and let it back in next turn.
#[test]
fn an_unnameable_member_is_withheld_and_reported_for_refetch() {
    let store = seed_store();
    store
        .replace_channel_members(
            "root",
            &[SELF_PK.into(), OTHER_PK.into(), GHOST_PK.into()],
            2,
        )
        .unwrap();
    say(&store, "msg-1", GHOST_PK, "root", 40);
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        !xml.lines()
            .find(|line| line.contains("<channel name=\"/root\""))
            .unwrap_or_default()
            .contains(" agents="),
        "an unclassified roster identity must make the count unknown: {xml}"
    );
    assert!(
        !member_rows(&xml).iter().any(|row| row.contains("ghost")),
        "{xml}"
    );
    assert_eq!(
        missing_profile_pubkeys(&captured(&store, &rec)),
        vec![GHOST_PK.to_string()]
    );

    // Once the kind:0 lands, the same member renders by name and stops being
    // reported — the refetch loop is self-terminating.
    store
        .upsert_profile_with_agent_slug(GHOST_PK, "ghost", "ghost", "ghost", "laptop", false, 3)
        .unwrap();
    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        xml.contains("<agent name=\"@ghost\" since=\"1 min ago\" />"),
        "{xml}"
    );
    assert!(missing_profile_pubkeys(&captured(&store, &rec)).is_empty());
}

/// A live status carries its own public slug, which is a usable name even before
/// the profile arrives. Such a member renders — but is still reported, because a
/// durable handle is worth having.
#[test]
fn a_status_slug_names_a_member_whose_profile_has_not_arrived() {
    let store = seed_store();
    store
        .replace_channel_members("root", &[SELF_PK.into(), GHOST_PK.into()], 2)
        .unwrap();
    heartbeat(&store, GHOST_PK, "amber-ghost", 95);
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(xml.contains("<agent name=\"@amber-ghost\""), "{xml}");
    assert_eq!(
        missing_profile_pubkeys(&captured(&store, &rec)),
        vec![GHOST_PK.to_string()],
        "a stopgap name is not a reason to stop looking for the real one"
    );
}

/// The report is a refetch worklist, so it must be deduped across channels and
/// must never ask the daemon to fetch identities it already owns.
#[test]
fn missing_profile_pubkeys_dedupes_and_excludes_self_and_backend() {
    let store = seed_store();
    for channel in ["root", TASK_H] {
        store
            .replace_channel_members(
                channel,
                &[SELF_PK.into(), GHOST_PK.into(), "mgmt-pubkey".into()],
                2,
            )
            .unwrap();
    }
    let rec = session(&store);
    let mut probe = input(Some(&rec), "root", 0, 100, true);
    probe.backend_pubkey = "mgmt-pubkey";
    let inputs = capture_inputs(&store, &probe).unwrap();

    assert_eq!(
        missing_profile_pubkeys(&inputs),
        vec![GHOST_PK.to_string()],
        "self is known, the mgmt key is ours, and a member on two channels is one fetch"
    );
}

/// Every member resolvable means nothing to fetch — the common steady state.
#[test]
fn missing_profile_pubkeys_is_empty_when_the_whole_roster_resolves() {
    let store = seed_store();
    let rec = session(&store);
    assert!(missing_profile_pubkeys(&captured(&store, &rec)).is_empty());
}

/// A profile row exists but carries no slug — a `kind:0` that resolved to
/// nothing useful. That is not a handle, and the member is treated as unnamed.
#[test]
fn a_blank_slug_does_not_count_as_a_handle() {
    let store = seed_store();
    store
        .replace_channel_members("root", &[SELF_PK.into(), GHOST_PK.into()], 2)
        .unwrap();
    store
        .upsert_profile(GHOST_PK, "Ghost", "   ", "laptop", false, 2)
        .unwrap();
    say(&store, "msg-1", GHOST_PK, "root", 40);
    let rec = session(&store);

    let inputs = captured(&store, &rec);
    assert!(!inputs.members.has_handle(GHOST_PK));
    assert_eq!(missing_profile_pubkeys(&inputs), vec![GHOST_PK.to_string()]);
}

/// The briefing RPC renders through the same path as a hook turn, so it reports
/// the same refetch worklist rather than silently rendering a short roster.
#[test]
fn the_full_session_briefing_hands_back_the_refetch_worklist() {
    let store = seed_store();
    store
        .replace_channel_members("root", &[SELF_PK.into(), GHOST_PK.into()], 2)
        .unwrap();
    say(&store, "msg-1", GHOST_PK, "root", 40);
    let rec = session(&store);

    let (fabric, missing) =
        render_full_session_state(&store, &rec, "coder", "", "laptop", 100).expect("briefing");
    assert!(
        !member_rows(&fabric).iter().any(|row| row.contains("ghost")),
        "{fabric}"
    );
    assert_eq!(missing, vec![GHOST_PK.to_string()]);
}
