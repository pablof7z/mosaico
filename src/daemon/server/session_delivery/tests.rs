use super::*;
use crate::state::{Profile, RegisterSession, TestGroup, TestGroupDelivery, TestRelayDelivery};

const PUBKEY: &str = "pi-pubkey";
const NATIVE_ID: &str = "pi-native-id";

fn caller() -> serde_json::Value {
    serde_json::json!({
        "harness": "pi",
        "harness_session": NATIVE_ID,
        "cwd": "/workspace",
    })
}

async fn seeded_state() -> (Arc<DaemonState>, crate::state::Session) {
    seeded_state_with_transport("").await
}

async fn seeded_state_with_transport(transport: &str) -> (Arc<DaemonState>, crate::state::Session) {
    let state = DaemonState::new_for_test().await;
    let session = state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(TestGroupDelivery::new([
                TestGroup::new("room").metadata("Room", "", "", 1)
            ]));
            store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles([Profile {
                pubkey: "peer-pubkey".into(),
                name: "peer".into(),
                slug: "peer".into(),
                agent_slug: String::new(),
                host: String::new(),
                is_backend: false,
                agents: Vec::new(),
                workspaces: Vec::new(),
                updated_at: 1,
            }]));
            store.reserve_session_with_facts(
                &RegisterSession {
                    pubkey: PUBKEY.into(),
                    observed_harness: "pi".into(),
                    agent_slug: "pi".into(),
                    launch_channel_h: "room".into(),
                    work_root: "room".into(),
                    child_pid: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "pi".into(),
                    claimed_harness: "pi".into(),
                    preset: String::new(),
                    transport: transport.into(),
                    endpoint_provenance: "hook".into(),
                },
            )?;
            store.set_native_resume_locator(PUBKEY, "pi", NATIVE_ID, 1)?;
            store.enqueue_inbox("event-one", PUBKEY, "peer-pubkey", "room", "Please help", 2)?;
            store.get_session(PUBKEY)?.context("missing Pi session")
        })
        .unwrap();
    (state, session)
}

#[tokio::test]
async fn lease_then_matching_ack_is_the_only_path_to_injected() {
    let (state, session) = seeded_state().await;
    let mut wait = caller();
    wait["timeout_secs"] = 1.into();
    let response = rpc_wait(&state, &wait).await.unwrap();
    let lease_id = response["lease_id"].as_str().unwrap().to_string();

    assert_eq!(response["kind"], "delivery");
    assert_eq!(response["message"]["custom_type"], "mosaico.delivery");
    assert_eq!(response["message"]["display"], false);
    assert!(response["message"]["content"]
        .as_str()
        .unwrap()
        .contains("Please help"));
    assert!(extension_delivery_live(&state, &session));
    let status = super::super::my_session::rpc_my_session(&state, &caller()).unwrap();
    assert!(
        !status["fabric"]
            .as_str()
            .unwrap()
            .contains("unhosted=\"true\""),
        "a live Pi extension must advertise its renewable delivery path"
    );
    state.with_store(|store| {
        assert_eq!(
            store.inbox_by_event_prefix("event-one").unwrap()[0].state,
            "leased"
        );
    });

    let mut ack = caller();
    ack["lease_id"] = lease_id.into();
    ack["accepted"] = true.into();
    let acknowledged = rpc_ack(&state, &ack).await.unwrap();
    assert_eq!(acknowledged["state"], "injected");
    assert_eq!(acknowledged["event_ids"], serde_json::json!(["event-one"]));
    state.with_store(|store| {
        assert_eq!(
            store.injected_for_pubkey(PUBKEY).unwrap()[0].event_id,
            "event-one"
        );
    });
}

#[tokio::test]
async fn foreign_or_rejected_ack_cannot_consume_a_leased_message() {
    let (state, _) = seeded_state().await;
    let mut wait = caller();
    wait["timeout_secs"] = 1.into();
    let delivery = rpc_wait(&state, &wait).await.unwrap();
    let lease_id = delivery["lease_id"].as_str().unwrap();

    let mut foreign = caller();
    foreign["harness_session"] = "other-native-id".into();
    foreign["lease_id"] = lease_id.into();
    foreign["accepted"] = true.into();
    assert!(rpc_ack(&state, &foreign).await.is_err());
    state.with_store(|store| {
        assert_eq!(
            store.inbox_by_event_prefix("event-one").unwrap()[0].state,
            "leased"
        );
    });

    let mut reject = caller();
    reject["lease_id"] = lease_id.into();
    reject["accepted"] = false.into();
    assert_eq!(rpc_ack(&state, &reject).await.unwrap()["state"], "requeued");
    state.with_store(|store| {
        assert_eq!(
            store.peek_pending_for_pubkey(PUBKEY).unwrap()[0].event_id,
            "event-one"
        );
    });
}

#[tokio::test]
async fn managed_pi_rpc_cannot_compete_with_the_extension_delivery_authority() {
    let (state, _) = seeded_state_with_transport("pi-rpc").await;
    let mut wait = caller();
    wait["timeout_secs"] = 1.into();
    let error = rpc_wait(&state, &wait).await.unwrap_err().to_string();
    assert!(
        error.contains("managed Pi RPC owns inbox delivery"),
        "{error}"
    );
}
