use super::*;
use crate::reconcile::InputFact;

fn fact(at: u64) -> DeliveryScanFact {
    DeliveryScanFact {
        session_id: "s1".into(),
        pending_event_ids: vec!["evt-1".into()],
        working: false,
        pty_id: Some("pty-1".into()),
        pty_live: true,
        last_injected_at: None,
        debounce_secs: 20,
        force: false,
        at,
    }
}

#[test]
fn idle_live_pty_with_pending_injects() {
    let mut r = DeliveryReconciler::new();
    let out = r.scan(fact(100)).unwrap();

    assert_eq!(
        out.effects,
        vec![DeliveryEffect::Inject {
            session_id: "s1".into(),
            pty_id: "pty-1".into(),
            event_ids: vec!["evt-1".into()],
        }]
    );
    r.assert_oracle().unwrap();
}

#[test]
fn debounced_pending_schedules_retry() {
    let mut r = DeliveryReconciler::new();
    let mut f = fact(116);
    f.last_injected_at = Some(100);

    let out = r.scan(f).unwrap();

    assert_eq!(
        out.effects,
        vec![DeliveryEffect::RetryAfter {
            session_id: "s1".into(),
            delay_secs: 4,
        }]
    );
    let row = r.state_rows().pop().unwrap();
    assert_eq!(row.action, "defer_debounced");
    assert_eq!(row.event_ids, vec!["evt-1"]);
    r.assert_oracle().unwrap();
}

#[test]
fn debounced_pending_becomes_injectable_after_retry_window() {
    let mut r = DeliveryReconciler::new();
    let mut blocked = fact(116);
    blocked.last_injected_at = Some(100);
    assert!(matches!(
        r.scan(blocked).unwrap().effects[0],
        DeliveryEffect::RetryAfter { .. }
    ));

    let mut retry = fact(120);
    retry.last_injected_at = Some(100);
    let out = r.scan(retry).unwrap();

    assert!(matches!(out.effects[0], DeliveryEffect::Inject { .. }));
    assert_eq!(r.state_rows()[0].action, "inject");
    r.assert_oracle().unwrap();
}

#[test]
fn working_session_defers_without_retry_timer() {
    let mut r = DeliveryReconciler::new();
    let mut f = fact(100);
    f.working = true;

    let out = r.scan(f).unwrap();

    assert!(out.effects.is_empty());
    assert_eq!(r.state_rows()[0].action, "defer_working");
    r.assert_oracle().unwrap();
}

#[test]
fn manual_force_bypasses_working_and_debounce() {
    let mut r = DeliveryReconciler::new();
    let mut f = fact(101);
    f.working = true;
    f.last_injected_at = Some(100);
    f.force = true;

    let out = r.scan(f).unwrap();

    assert!(matches!(out.effects[0], DeliveryEffect::Inject { .. }));
    r.assert_oracle().unwrap();
}

#[test]
fn replay_capsule_accepts_delivery_scan_fact() {
    let mut script = trellis_testing::DataTransactionScript::new();
    script
        .step("delivery/scan")
        .operation(InputFact::DeliveryScan(fact(100)))
        .commit();

    let report = crate::reconcile::replay::replay_script(&script, false).unwrap();

    assert_eq!(report.surface, "delivery");
    assert_eq!(report.steps, 1);
    assert_eq!(report.resource_commands, 1);
}
