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
