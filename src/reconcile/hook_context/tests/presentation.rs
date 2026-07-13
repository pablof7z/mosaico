use super::HookContextReconciler;

#[test]
fn headless_mode_notice_is_emitted_only_for_a_transition() {
    let mut reconciler = HookContextReconciler::new();
    assert!(reconciler.record_headless_mode(false));
    assert!(!reconciler.record_headless_mode(false));
    assert!(reconciler.record_headless_mode(true));
    assert!(!reconciler.record_headless_mode(true));
}
