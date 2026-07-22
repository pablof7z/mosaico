use super::*;
use crate::state::{RecordMessage, RegisterSession};

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
        .upsert_profile(pubkey, slug, slug, "test-host", false, now)
        .unwrap();
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "codex".into(),
            agent_slug: slug.into(),
            channel_h: "root".into(),
            child_pid: None,
            transcript_path: None,
            now,
        })
        .unwrap()
}

fn record(store: &crate::state::Store, id: usize, author: &str, body: String, at: u64) {
    store
        .record_message(&RecordMessage {
            message_id: format!("message-{id}"),
            thread_id: "root".into(),
            channel_h: "root".into(),
            author_pubkey: author.into(),
            body,
            created_at: at,
            direction: "inbound".into(),
            sync_state: "accepted".into(),
            native_event_id: None,
            error: None,
        })
        .unwrap();
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
fn retry_can_continue_after_the_creator_already_switched_to_the_offered_child() {
    assert!(caller_can_resume_offer("root", "root", Some("child")));
    assert!(caller_can_resume_offer("child", "root", Some("child")));
    assert!(!caller_can_resume_offer("other", "root", Some("child")));
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
        "root",
        "focused",
        "Coordinate the focused implementation",
    );

    assert_eq!(created["parent"], "root");
    assert_eq!(created["name"], "focused");
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
        store.upsert_channel("root", "root", "", "", now).unwrap();
        store
            .upsert_channel("child", "focused", "", "root", now)
            .unwrap();
        let a1_generation = seed_session(store, A1, "a1", now.saturating_sub(60));
        seed_session(store, A2, "a2", now.saturating_sub(60));
        store
            .apply_session_turn_started(A1, a1_generation, now, None)
            .unwrap();
        store
            .replace_channel_members("child", &[A1.into(), A2.into()], now)
            .unwrap();
        for (id, author, ago) in [
            (1, A1, 30),
            (2, A2, 25),
            (3, A1, 20),
            (4, A2, 15),
            (5, A1, 10),
            (6, A2, 5),
        ] {
            record(
                store,
                id,
                author,
                format!("substantive coordination message {id}"),
                now.saturating_sub(ago),
            );
        }
        record(store, 7, A1, "Moving this to #focused".into(), now);
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
        assert_eq!(store.get_session(A1).unwrap().unwrap().channel_h, "child");
        assert_eq!(store.get_session(A2).unwrap().unwrap().channel_h, "root");
        assert!(store.has_session_route(A2, "child").unwrap());
    });
}
