use super::*;
use crate::state::{
    Profile, RegisterSession, RelayEvent, TestGroup, TestGroupDelivery, TestRelayDelivery,
};

const A1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const A2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn participant(pubkey: &str, generation: Option<u64>) -> ParticipantSnapshot {
    ParticipantSnapshot {
        pubkey: pubkey.into(),
        label: pubkey.into(),
        host: "host".into(),
        runtime_generation: generation,
        live: true,
        busy: false,
    }
}

fn evidence(cohort: Vec<ParticipantSnapshot>) -> ConversationEvidence {
    ConversationEvidence {
        parent: "root".into(),
        cohort,
        busy_pubkeys: vec!["a".into()],
        audience_count: 2,
        engaged_count: 2,
        message_count: 6,
        alternations: 5,
        started_at: 1,
        ended_at: 30,
        last_message_id: "m".into(),
    }
}

fn seed_session(store: &crate::state::Store, pubkey: &str, slug: &str, now: u64) -> u64 {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "codex".into(),
            agent_slug: slug.into(),
            launch_channel_h: "root".into(),
            work_root: "root".into(),
            child_pid: None,
            now,
        })
        .unwrap()
}

fn profile(pubkey: &str, slug: &str, at: u64) -> Profile {
    Profile {
        pubkey: pubkey.into(),
        name: slug.into(),
        slug: slug.into(),
        agent_slug: String::new(),
        host: "test-host".into(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: at,
    }
}

fn record(id: usize, author: &str, body: String, at: u64) -> RelayEvent {
    RelayEvent {
        id: format!("message-{id}"),
        kind: 9,
        pubkey: author.into(),
        created_at: at,
        channel_h: "root".into(),
        d_tag: String::new(),
        content: body,
        tags_json: "[]".into(),
    }
}

#[test]
fn stale_generation_or_added_speaker_invalidates_offer() {
    let captured = evidence(vec![participant("a", Some(1)), participant("b", None)]);
    assert!(same_cohort(&captured, &captured));
    assert!(!same_cohort(
        &captured,
        &evidence(vec![participant("a", Some(2)), participant("b", None)])
    ));
    assert!(!same_cohort(
        &captured,
        &evidence(vec![
            participant("a", Some(1)),
            participant("b", None),
            participant("c", None),
        ])
    ));
}

#[test]
fn move_creation_uses_the_required_about_as_the_child_about() {
    let params = serde_json::json!({
        "name": "focused",
        "about": "Coordinate the focused implementation",
        "session": A1,
    });
    let created = move_create_params(
        &params,
        "#root",
        "focused",
        "Coordinate the focused implementation",
    );

    assert_eq!(created["channel"], "#root/focused");
    assert_eq!(created["about"], "Coordinate the focused implementation");
    assert_eq!(created["agents"], serde_json::json!([]));
    assert_eq!(created["session"], A1);
}

#[tokio::test]
async fn accepting_validates_the_about_before_offer_lookup() {
    let state = DaemonState::new_for_test().await;
    let error = rpc_accept(
        &state,
        &serde_json::json!({ "name": "focused", "about": "   ", "session": A1 }),
    )
    .await
    .expect_err("empty about must be rejected");

    assert!(format!("{error:#}").contains("requires a non-empty channel about"));

    let too_long = "x".repeat(crate::channel_about::CHANNEL_ABOUT_MAX_CHARS + 1);
    let error = rpc_accept(
        &state,
        &serde_json::json!({ "name": "focused", "about": too_long, "session": A1 }),
    )
    .await
    .expect_err("overlong about must be rejected");

    assert!(format!("{error:#}").contains("80 characters or fewer"));
}

#[tokio::test]
async fn accepting_reuses_child_focuses_caller_and_passively_adds_idle_peer() {
    let state = DaemonState::new_for_test().await;
    let now = now_secs();
    state.with_store(|store| {
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([
            TestGroup::new("root").metadata("root", "", "", now),
            TestGroup::new("child")
                .metadata("focused", "", "root", now)
                .members(vec![A1.into(), A2.into()]),
        ]));
        let a1_generation = seed_session(store, A1, "a1", now.saturating_sub(60));
        seed_session(store, A2, "a2", now.saturating_sub(60));
        store
            .apply_session_turn_started(A1, a1_generation, now)
            .unwrap();
        let mut events = Vec::new();
        for (id, author, ago) in [
            (1, A1, 30),
            (2, A2, 25),
            (3, A1, 20),
            (4, A2, 15),
            (5, A1, 10),
            (6, A2, 5),
        ] {
            events.push(record(
                id,
                author,
                format!("substantive coordination message {id}"),
                now.saturating_sub(ago),
            ));
        }
        events.push(record(
            7,
            A1,
            "Continue this conversation in #root/focused; existing channel memberships are unchanged"
                .into(),
            now,
        ));
        store.install_test_nmp_relay_delivery(
            TestRelayDelivery::new()
                .profiles([
                    profile(A1, "a1", now.saturating_sub(60)),
                    profile(A2, "a2", now.saturating_sub(60)),
                ])
                .events(events),
        );
    });

    let captured = current_evidence(&state, "root", now)
        .unwrap()
        .expect("conversation qualifies");
    state
        .runtime
        .channel_nudges
        .lock()
        .unwrap()
        .consider(A1, captured, now, 0)
        .expect("winning caller receives an offer");

    let response = rpc_accept(
        &state,
        &serde_json::json!({
            "name": "focused",
            "about": "Coordinate the focused implementation",
            "session": A1,
        }),
    )
    .await
    .expect("acceptance should reuse the ready child");
    assert_eq!(response["created"], false);
    assert_eq!(response["added"], serde_json::json!([A1, A2]));
    assert_eq!(response["pointer_posted"], false);
    assert_eq!(response["child_seed_posted"], false);

    state.with_store(|store| {
        assert!(store.has_session_route(A1, "child").unwrap());
        assert!(store.has_session_route(A2, "root").unwrap());
        assert!(store.has_session_route(A2, "child").unwrap());
    });
}
