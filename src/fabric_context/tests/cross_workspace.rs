use super::*;
use crate::reconcile::hook_context::HookContextState;

fn add_workspace(store: &Store) {
    store
        .upsert_channel("remote", "general", "Remote room", "", 1)
        .unwrap();
    store
        .upsert_channel("review", "review", "Review room", "remote", 1)
        .unwrap();
}

fn put_status(
    store: &Store,
    pubkey: &str,
    channel: &str,
    activity: &str,
    updated_at: u64,
    expiration: u64,
) {
    store
        .upsert_status(&Status {
            pubkey: pubkey.into(),
            channel_h: channel.into(),
            slug: "reviewer".into(),
            title: "Reviewing".into(),
            activity: activity.into(),
            state: crate::session_state::SessionState::Working,
            state_since: updated_at,
            last_seen: updated_at,
            updated_at,
            expiration,
        })
        .unwrap();
}

#[test]
fn delta_includes_other_workspace_root_and_descendant_presence_only() {
    let store = seed_store();
    let rec = session(&store);
    add_workspace(&store);
    put_status(&store, OTHER_PK, "remote", "coordinating release", 250, 500);
    put_status(&store, OTHER_PK, "review", "reviewing patch", 260, 500);
    chat(
        &store,
        "remote-chat",
        "review",
        270,
        "private unjoined chatter",
        "[]",
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("other activity should render");
    assert_eq!(
        text.matches("<workspace name=\"remote\"").count(),
        1,
        "{text}"
    );
    assert!(text.contains("<workspace name=\"remote\" channel=\"remote\""));
    assert!(text.contains("text=\"coordinating release\""), "{text}");
    assert!(text.contains("<channel name=\"#review\" ref=\"remote.review\""));
    assert!(text.contains("text=\"reviewing patch\""), "{text}");
    assert!(!text.contains("private unjoined chatter"), "{text}");

    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 200, 300)),
        text
    );
    let mut state = HookContextState::default();
    let outcome = state.render_context("sess", "turn_start", 200, 300, captured);
    assert_eq!(outcome.text.as_deref(), Some(text.as_str()));
    let human =
        render_fabric_context_human(&store, input(Some(&rec), "root", 200, 300, false), false)
            .expect("valid channel ancestry")
            .expect("human delta");
    assert!(human.contains("remote\nRemote room"), "{human}");
    assert!(human.contains("#remote.review"), "{human}");
    assert!(human.contains("reviewing patch"), "{human}");

    let full = render_fabric_context(&store, input(Some(&rec), "root", 0, 300, false))
        .expect("current workspace full snapshot");
    assert!(!full.contains("<workspace name=\"remote\""), "{full}");
}

#[test]
fn other_workspace_delta_reports_expiry_once_and_rejects_stale_and_self_statuses() {
    let store = seed_store();
    let rec = session(&store);
    add_workspace(&store);
    put_status(&store, OTHER_PK, "remote", "old work", 150, 500);
    put_status(&store, OTHER_PK, "remote", "expired work", 250, 299);
    put_status(&store, SELF_PK, "remote", "self work", 250, 500);
    put_status(&store, OTHER_PK, "root", "current workspace work", 250, 500);

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("current workspace activity should render");
    assert!(text.contains("current workspace work"), "{text}");
    assert!(text.contains("<workspace name=\"remote\""), "{text}");
    assert!(!text.contains("old work"), "{text}");
    assert!(text.contains("state=\"offline\""), "{text}");
    assert!(!text.contains("expired work"), "{text}");
    assert!(!text.contains("self work"), "{text}");
}
