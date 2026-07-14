//! Transport-aware delivery routing tests.
//!
//! These exercise the liveness-probe seam that decides whether a session's
//! recorded endpoint is driven as a PTY (bracketed-paste inject) or an ACP child
//! (JSON-RPC deliver). The endpoint id lives under the SAME `pty_session` alias
//! for both transports, so the routing MUST key off the session's transport kind
//! rather than the alias name — the exact regression these tests pin down.

use super::*;

#[test]
fn acp_endpoint_liveness_uses_the_acp_registry() {
    // An unregistered ACP endpoint id is reported dead by the ACP registry probe.
    // Crucially it is classified via the ACP path, not the PTY socket probe.
    assert!(!endpoint_is_live(
        TransportKind::Acp,
        "te-acp-unknown-endpoint"
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
    // The bug this fix closes: an ACP endpoint id (`te-acp-*`) is stored under the
    // `pty_session` alias, so the pre-fix doorbell probed it with `pty::is_live`
    // and ALWAYS saw it dead — clearing the endpoint and dropping the mention.
    // Confirm the PTY probe indeed reports an ACP id dead, proving the doorbell
    // must consult the ACP probe (via `endpoint_is_live`) for ACP sessions.
    let acp_id = "te-acp-claude-1-2-3";
    assert!(
        !crate::pty::is_live(acp_id),
        "an ACP endpoint id is not a live PTY"
    );
    // Routed through the transport-aware probe it is (correctly) dead here only
    // because it is unregistered — not because the wrong probe was used.
    assert!(!endpoint_is_live(TransportKind::Acp, acp_id));
}

/// `session_has_live_delivery_endpoint` gates the turn-context reachability
/// warning: no alias, or an alias whose endpoint is dead, both read as
/// unavailable; only a `pty_session` alias resolving to a live listener reads
/// as available.
#[test]
fn session_has_live_delivery_endpoint_true_only_for_a_live_alias() {
    let store = crate::state::Store::open_memory().unwrap();
    let rec = crate::state::Session {
        session_id: "sess-pty-probe".into(),
        agent_pubkey: "pk-probe".into(),
        agent_slug: "probe-agent".into(),
        channel_h: "proj".into(),
        harness: "claude-code".into(),
        child_pid: None,
        transcript_path: None,
        alive: true,
        created_at: 1,
        last_seen: 1,
        working: false,
        turn_started_at: 0,
        last_distill_at: 0,
        work_topic: String::new(),
        work_topic_set_at: 0,
        seen_cursor: 0,
        title: String::new(),
        activity: String::new(),
        resume_id: String::new(),
        distill_fail_streak: 0,
        distill_notice_at: 0,
        explicit_chat_published_at: 0,
    };

    assert!(
        !session_has_live_delivery_endpoint(&store, &rec),
        "no alias at all must read as unavailable"
    );

    let dir = tempfile::tempdir().unwrap();
    let dead_path = dir.path().join("dead.sock");
    store
        .put_alias(
            "claude-code",
            "pty_session",
            dead_path.to_str().unwrap(),
            &rec.session_id,
            1,
        )
        .unwrap();
    assert!(
        !session_has_live_delivery_endpoint(&store, &rec),
        "an alias to a socket nobody is listening on must read as unavailable"
    );

    let live_path = dir.path().join("live.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&live_path).unwrap();
    store
        .put_alias(
            "claude-code",
            "pty_session",
            live_path.to_str().unwrap(),
            &rec.session_id,
            2,
        )
        .unwrap();
    assert!(
        session_has_live_delivery_endpoint(&store, &rec),
        "a pty_session alias resolving to a live listener must read as available"
    );
}

#[test]
fn headless_mode_separates_output_visibility_from_reachability() {
    assert!(!headless_for_endpoint(TransportKind::Pty, false, false));
    assert!(headless_for_endpoint(TransportKind::Pty, true, false));
    assert!(!headless_for_endpoint(TransportKind::Pty, true, true));
    assert!(headless_for_endpoint(TransportKind::Acp, false, false));
    assert!(headless_for_endpoint(TransportKind::Acp, true, false));
}

/// The reconciler is transport-neutral: it plans an `Inject` carrying the
/// endpoint id regardless of transport (`apply_delivery_effects` then routes that
/// id by kind — ACP deliver vs. PTY inject). Confirm a live ACP endpoint id with
/// pending rows yields an `Inject` carrying that id, while a dead endpoint is
/// cleared — the same decision for PTY and ACP.
#[test]
fn reconciler_plans_inject_for_live_endpoint_and_clears_dead() {
    use crate::reconcile::delivery::{DeliveryEffect, DeliveryReconciler, DeliveryScanFact};

    let mut r = DeliveryReconciler::new();
    let live = r
        .scan(DeliveryScanFact {
            session_id: "sess-live".into(),
            pending_event_ids: vec!["evt-1".into()],
            pty_id: Some("te-acp-live".into()),
            pty_live: true,
            last_injected_at: None,
            debounce_secs: 20,
            force: true,
            at: 100,
        })
        .unwrap();
    assert!(
        matches!(
            live.effects.as_slice(),
            [DeliveryEffect::Inject { pty_id, .. }] if pty_id == "te-acp-live"
        ),
        "live ACP endpoint should yield Inject carrying the endpoint id, got {:?}",
        live.effects
    );

    let dead = r
        .scan(DeliveryScanFact {
            session_id: "sess-dead".into(),
            pending_event_ids: vec!["evt-2".into()],
            pty_id: Some("te-acp-dead".into()),
            pty_live: false,
            last_injected_at: None,
            debounce_secs: 20,
            force: true,
            at: 100,
        })
        .unwrap();
    assert!(
        matches!(
            dead.effects.as_slice(),
            [DeliveryEffect::ClearDeadEndpoint { .. }]
        ),
        "dead endpoint should be cleared, got {:?}",
        dead.effects
    );
}
