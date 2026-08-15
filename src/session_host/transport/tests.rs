use super::*;

#[path = "tests/kinds.rs"]
mod kinds;
#[path = "tests/pi.rs"]
mod pi;

#[test]
fn persisted_locator_selects_the_transport_without_agent_config() {
    for kind in TransportKind::ALL {
        let locator = crate::state::SessionLocator {
            harness: "codex".into(),
            locator_kind: kind.locator_kind().into(),
            locator_value: format!("{}-owned-endpoint", kind.as_str()),
            pubkey: "pk".into(),
            runtime_generation: 0,
            created_at: 1,
        };
        let (transport, endpoint) = transport_for_locator(&locator).expect("hosted locator");
        assert_eq!(transport.kind(), kind);
        assert_eq!(endpoint.kind, kind);
        assert_eq!(endpoint.endpoint_id, locator.locator_value);
    }
}

#[test]
fn admitted_hosted_transport_remains_distinct_when_locator_is_missing() {
    for kind in TransportKind::ALL {
        let store = crate::state::Store::open_memory().unwrap();
        let pubkey = format!("pk-missing-{}", kind.as_str());
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: pubkey.clone(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    launch_channel_h: "root".into(),
                    work_root: "root".into(),
                    child_pid: Some(std::process::id() as i32),
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    preset: String::new(),
                    transport: kind.as_str().into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();
        let session = store.get_session(&pubkey).unwrap().unwrap();
        match hosted_endpoint_for(&store, &session).unwrap() {
            HostedEndpoint::Unavailable { kind: actual } => assert_eq!(actual, kind),
            HostedEndpoint::Unhosted | HostedEndpoint::Resolved { .. } => {
                panic!("missing {} locator lost admitted transport", kind.as_str())
            }
        }
    }
}

#[test]
fn transport_selection_is_direct() {
    for (transport, kind) in [
        (Transport::Pty, TransportKind::Pty),
        (Transport::Acp, TransportKind::Acp),
        (Transport::AppServer, TransportKind::AppServer),
        (Transport::PiRpc, TransportKind::PiRpc),
    ] {
        assert_eq!(select_transport(transport).kind(), kind);
        assert_eq!(transport_kind_for(transport), kind);
    }
}
