use super::*;
use crate::state::{AdmittedRuntimeFacts, Profile, RegisterSession, StopReason};

fn seed_stopped_member(store: &Store) {
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 900)
    ]));
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles([Profile {
        pubkey: "pk-codex".into(),
        name: "willow-summit-042-codex".into(),
        slug: "willow-summit-042".into(),
        agent_slug: "codex".into(),
        host: "laptop".into(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: 900,
    }]));
    let generation = store
        .reserve_session_with_facts(
            &RegisterSession {
                pubkey: "pk-codex".into(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                launch_channel_h: "proj".into(),
                work_root: "proj".into(),
                child_pid: None,
                now: 900,
            },
            &AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: String::new(),
                preset: String::new(),
                transport: "app-server".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    let session = store.get_session("pk-codex").unwrap().unwrap();
    assert_eq!(
        store
            .commit_confirmed_session_admission(
                "pk-codex",
                "proj",
                generation,
                session.lifecycle_epoch,
                900,
            )
            .unwrap(),
        crate::state::ConfirmedAdmissionCommit::Committed
    );
    store
        .mark_runtime_stopped_if_generation("pk-codex", generation, StopReason::HeadlessExit, 900)
        .unwrap();
}

#[test]
fn stopped_session_keeps_membership_as_dormant_presence() {
    let store = Store::open_memory().unwrap();
    seed_stopped_member(&store);

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let row = snapshot.rows.first().expect("dormant row");
    assert!(row.dormant);
    assert_eq!(row.slug, "codex");
    assert_eq!(row.age_secs, Some(100));
    assert!(!row.remote);
    assert!(store.has_session_route("pk-codex", "proj").unwrap());
    assert_eq!(
        store
            .get_session_standing("pk-codex", "proj")
            .unwrap()
            .unwrap()
            .state,
        crate::state::StandingState::Member
    );
}

#[test]
fn stopped_membership_does_not_expire() {
    let store = Store::open_memory().unwrap();
    seed_stopped_member(&store);
    let snapshot = load_who_snapshot(&store, Some("proj"), 4_501, "laptop").unwrap();
    let row = snapshot.rows.first().expect("dormant row");
    assert!(row.dormant);
    assert_eq!(row.age_secs, Some(3_601));
    assert!(store.has_session_route("pk-codex", "proj").unwrap());
}
