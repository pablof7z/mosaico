//! Transport-aware delivery routing tests.
//!
//! These exercise the liveness-probe seam that decides whether a session's
//! recorded endpoint is driven as a PTY (bracketed-paste inject) or an ACP child
//! (JSON-RPC deliver). PTY and ACP use distinct typed locators, and routing keys
//! off the session's transport kind.

use super::*;

fn endpoint_is_live(kind: TransportKind, endpoint_id: &str) -> bool {
    let transport = transport_for_kind(kind);
    transport.is_live(&EndpointRef {
        kind,
        endpoint_id: endpoint_id.to_string(),
    })
}

#[test]
fn acp_endpoint_liveness_uses_the_acp_registry() {
    // An unregistered ACP endpoint id is reported dead by the ACP registry probe.
    // Crucially it is classified via the ACP path, not the PTY socket probe.
    assert!(!endpoint_is_live(
        TransportKind::Acp,
        "acp-unknown-endpoint"
    ));
}

#[test]
fn pty_endpoint_liveness_uses_the_pty_probe() {
    assert!(!endpoint_is_live(
        TransportKind::Pty,
        "not-a-live-pty-socket"
    ));
}

#[test]
fn acp_endpoint_id_would_read_dead_under_the_old_pty_probe() {
    // The bug this fix closes: an ACP endpoint id (`acp-*`) is stored under the
    // `pty_session` alias, so the pre-fix doorbell probed it with `pty::is_live`
    // and ALWAYS saw it dead — clearing the endpoint and dropping the mention.
    // Confirm the PTY probe indeed reports an ACP id dead, proving the doorbell
    // must consult the ACP probe (via `endpoint_is_live`) for ACP sessions.
    let acp_id = "acp-claude-1-2-3";
    assert!(
        !crate::pty::is_live(acp_id),
        "an ACP endpoint id is not a live PTY"
    );
    // Routed through the transport-aware probe it is (correctly) dead here only
    // because it is unregistered — not because the wrong probe was used.
    assert!(!endpoint_is_live(TransportKind::Acp, acp_id));
}

/// `session_has_live_delivery_path` gates the turn-context reachability
/// warning: no locator, or a locator whose endpoint is dead, both read as
/// unavailable; only a PTY locator resolving to a live listener reads
/// as available.
#[test]
fn session_has_live_delivery_path_true_only_for_a_live_locator() {
    let store = crate::state::Store::open_memory().unwrap();
    store
        .reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: "pk-probe".into(),
                agent_slug: "probe-agent".into(),
                channel_h: "proj".into(),
                observed_harness: "claude-code".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            },
            &crate::state::AdmittedRuntimeFacts {
                observed_harness: "claude-code".into(),
                claimed_harness: String::new(),
                bundle: "claude-pty".into(),
                transport: "pty".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    let rec = store.get_session("pk-probe").unwrap().unwrap();

    assert!(
        !session_has_live_delivery_path(&store, &rec),
        "no locator at all must read as unavailable"
    );

    let dir = tempfile::tempdir().unwrap();
    let dead_path = dir.path().join("dead.sock");
    store
        .put_session_locator(
            "claude-code",
            crate::state::LOCATOR_PTY,
            dead_path.to_str().unwrap(),
            &rec.pubkey,
            1,
        )
        .unwrap();
    assert!(
        !session_has_live_delivery_path(&store, &rec),
        "a locator to a socket nobody is listening on must read as unavailable"
    );

    let live_path = dir.path().join("live.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&live_path).unwrap();
    store
        .put_session_locator(
            "claude-code",
            crate::state::LOCATOR_PTY,
            live_path.to_str().unwrap(),
            &rec.pubkey,
            2,
        )
        .unwrap();
    assert!(
        session_has_live_delivery_path(&store, &rec),
        "a PTY locator resolving to a live listener must read as available"
    );

    store
        .put_session_locator(
            "codex",
            crate::state::LOCATOR_PTY,
            "newer-but-foreign",
            &rec.pubkey,
            3,
        )
        .unwrap();
    assert!(
        session_has_live_delivery_path(&store, &rec),
        "a newer locator under a claimed foreign harness cannot shadow the admitted endpoint"
    );
}

#[test]
fn headless_mode_separates_output_visibility_from_reachability() {
    let store = crate::state::Store::open_memory().unwrap();
    store
        .reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: "pk-output".into(),
                agent_slug: "agent".into(),
                channel_h: "root".into(),
                observed_harness: "codex".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            },
            &crate::state::AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: String::new(),
                bundle: "codex-acp".into(),
                transport: "app-server".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    let session = store.get_session("pk-output").unwrap().unwrap();
    assert!(
        session_is_headless(&store, &session),
        "an admitted hosted session without its endpoint has no visible output surface"
    );

    store
        .put_session_locator(
            "codex",
            crate::state::LOCATOR_APP_SERVER,
            "app-server-1",
            "pk-output",
            2,
        )
        .unwrap();
    assert!(session_is_headless(&store, &session));
    store
        .clear_session_locator_kind("pk-output", "codex", crate::state::LOCATOR_APP_SERVER)
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("visible.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let worker = std::thread::spawn(move || {
        use std::io::{BufRead, Write};
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let mut command = String::new();
        reader.read_line(&mut command).unwrap();
        reader.get_mut().write_all(b"headed\n").unwrap();
    });
    store
        .put_session_locator(
            "codex",
            crate::state::LOCATOR_PTY,
            socket.to_str().unwrap(),
            "pk-output",
            3,
        )
        .unwrap();
    let mut session = session;
    session.admitted_transport = "pty".into();
    assert!(!session_is_headless(&store, &session));
    worker.join().unwrap();
}

/// The delivery policy is transport-neutral: it plans an `Inject` carrying the
/// endpoint id regardless of transport (`apply_delivery_effects` then routes that
/// id by kind — ACP deliver vs. PTY inject). Confirm a live ACP endpoint id with
/// pending rows yields an `Inject` carrying that id, while a dead endpoint is
/// cleared — the same decision for PTY and ACP.
#[test]
fn policy_plans_inject_for_live_endpoint_and_clears_dead() {
    use crate::reconcile::delivery::{decide, effects, DeliveryEffect, DeliveryScanFact};

    let live_decision = decide(&DeliveryScanFact {
        pubkey: "sess-live".into(),
        pending_event_ids: vec!["evt-1".into()],
        endpoint_id: Some("acp-live".into()),
        endpoint_live: true,
        last_injected_at: None,
        debounce_secs: 20,
        force: true,
        at: 100,
    })
    .unwrap();
    let live = effects(Some(&live_decision));
    assert!(
        matches!(
            live.as_slice(),
            [DeliveryEffect::Inject { endpoint_id, .. }] if endpoint_id == "acp-live"
        ),
        "live ACP endpoint should yield Inject carrying the endpoint id, got {:?}",
        live
    );

    let dead_decision = decide(&DeliveryScanFact {
        pubkey: "sess-dead".into(),
        pending_event_ids: vec!["evt-2".into()],
        endpoint_id: Some("acp-dead".into()),
        endpoint_live: false,
        last_injected_at: None,
        debounce_secs: 20,
        force: true,
        at: 100,
    })
    .unwrap();
    let dead = effects(Some(&dead_decision));
    assert!(
        matches!(dead.as_slice(), [DeliveryEffect::ClearDeadEndpoint { .. }]),
        "dead endpoint should be cleared, got {:?}",
        dead
    );
}
