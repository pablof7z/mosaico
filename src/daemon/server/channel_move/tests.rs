use super::*;
use crate::state::{
    Profile, RegisterSession, RelayEvent, StopReason, TestGroup, TestGroupDelivery,
    TestRelayDelivery,
};

const A1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const A2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const A3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const SILENT: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const STOPPED: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const HUMAN: &str = "6666666666666666666666666666666666666666666666666666666666666666";
const BACKEND: &str = "7777777777777777777777777777777777777777777777777777777777777777";

fn seed_session(store: &crate::state::Store, pubkey: &str, slug: &str) -> u64 {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "codex".into(),
            agent_slug: slug.into(),
            launch_channel_h: "root".into(),
            work_root: "root".into(),
            child_pid: None,
            now: 1,
        })
        .unwrap()
}

fn profile(pubkey: &str, slug: &str, is_backend: bool) -> Profile {
    Profile {
        pubkey: pubkey.into(),
        name: slug.into(),
        slug: slug.into(),
        agent_slug: String::new(),
        host: "test-host".into(),
        is_backend,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: 1,
    }
}

fn event(id: &str, kind: u16, author: &str, body: &str, at: u64, tags: &str) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind: kind as u32,
        pubkey: author.into(),
        created_at: at,
        channel_h: "root".into(),
        d_tag: String::new(),
        content: body.into(),
        tags_json: tags.into(),
    }
}

#[tokio::test]
async fn store_adapter_separates_conversation_busy_and_non_agent_audiences() {
    let state = DaemonState::new_for_test_with_whitelisted(vec![HUMAN.into()]).await;
    state.with_store(|store| {
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([
            TestGroup::new("root").metadata("root", "", "", 1),
            TestGroup::new("child").metadata("child", "", "root", 2),
        ]));

        let a1_generation = seed_session(store, A1, "a1");
        let a2_generation = seed_session(store, A2, "a2");
        seed_session(store, A3, "a3");
        seed_session(store, SILENT, "silent");
        let stopped_generation = seed_session(store, STOPPED, "stopped");
        seed_session(store, HUMAN, "human");
        seed_session(store, BACKEND, "backend");

        store
            .apply_session_turn_started(A1, a1_generation, 900)
            .unwrap();
        store
            .apply_session_turn_started(A2, a2_generation, 901)
            .unwrap();
        store
            .mark_runtime_stopped_if_generation(
                STOPPED,
                stopped_generation,
                StopReason::Unknown,
                902,
            )
            .unwrap();

        let mut events = Vec::new();
        for (id, author, at) in [
            (1, A1, 800),
            (2, A2, 805),
            (3, A3, 810),
            (4, A1, 815),
            (5, A2, 820),
            (6, A3, 825),
            (7, STOPPED, 830),
            (8, HUMAN, 835),
            (9, BACKEND, 840),
        ] {
            events.push(event(
                &format!("message-{id}"),
                9,
                author,
                &format!("substantive coordination message {id}"),
                at,
                "[]",
            ));
        }
        events.push(event(
            "reaction-silent",
            7,
            SILENT,
            "👀",
            850,
            r#"[["e","message-1"]]"#,
        ));
        events.push(event(
            "reaction-human",
            7,
            HUMAN,
            "👍",
            851,
            r#"[["e","message-1"]]"#,
        ));
        store.install_test_nmp_relay_delivery(
            TestRelayDelivery::new()
                .profiles([
                    profile(A1, "a1", false),
                    profile(A2, "a2", false),
                    profile(A3, "a3", false),
                    profile(SILENT, "silent", false),
                    profile(STOPPED, "stopped", false),
                    profile(HUMAN, "human", false),
                    profile(BACKEND, "backend", true),
                ])
                .events(events),
        );
    });

    let evidence = current_evidence(&state, "root", 1_000)
        .unwrap()
        .expect("root conversation should qualify");
    assert_eq!(
        evidence
            .cohort
            .iter()
            .map(|participant| participant.pubkey.as_str())
            .collect::<Vec<_>>(),
        [A1, A2, A3]
    );
    assert_eq!(evidence.busy_pubkeys, [A1, A2]);
    assert_eq!(
        evidence.audience_count, 4,
        "silent live agent counts only in audience"
    );
    assert_eq!(
        evidence.engaged_count, 4,
        "a live reactor is engaged but stays outside the speaking cohort"
    );
    let caller = state
        .with_store(|store| store.get_session(A1))
        .unwrap()
        .unwrap();
    let nudge = maybe_nudge_with_roll(&state, &caller, 1_000, 0)
        .expect("a winning BUSY caller should receive the nudge");
    assert!(nudge.contains("--yes-lets-move"));
    assert!(current_evidence(&state, "child", 1_000).unwrap().is_none());
}
