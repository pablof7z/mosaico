use super::HookContextReconciler;

#[test]
fn headless_mode_notice_emits_on_turn_start_or_a_later_transition() {
    let mut reconciler = HookContextReconciler::new();
    assert!(!reconciler.record_headless_mode(false, false));
    assert!(!reconciler.record_headless_mode(false, false));
    assert!(reconciler.record_headless_mode(true, false));
    assert!(!reconciler.record_headless_mode(true, false));

    let mut at_turn_start = HookContextReconciler::new();
    assert!(at_turn_start.record_headless_mode(false, true));
}
