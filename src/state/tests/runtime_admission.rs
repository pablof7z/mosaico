use super::*;

fn facts(
    observed: &str,
    claimed: &str,
    bundle: &str,
    transport: &str,
    provenance: &str,
) -> AdmittedRuntimeFacts {
    AdmittedRuntimeFacts {
        observed_harness: observed.into(),
        claimed_harness: claimed.into(),
        bundle: bundle.into(),
        transport: transport.into(),
        endpoint_provenance: provenance.into(),
    }
}

#[test]
fn hook_claim_cannot_rewrite_launch_facts_even_after_the_row_is_dead() {
    let store = Store::open_memory().unwrap();
    let registration = reg("grok", "pk", "room");
    store
        .reserve_session_with_facts(
            &registration,
            &facts("grok", "", "grok-pty", "pty", "launch"),
        )
        .unwrap();
    store.mark_dead("pk").unwrap();

    let hook_registration = RegisterSession {
        observed_harness: "claude-code".into(),
        now: 2_000,
        ..registration
    };
    store
        .reserve_session_with_facts(
            &hook_registration,
            &facts("claude-code", "claude-code", "", "pty", "hook"),
        )
        .unwrap();

    let session = store.get_session("pk").unwrap().unwrap();
    assert_eq!(session.observed_harness, "grok");
    assert_eq!(session.claimed_harness, "claude-code");
    assert_eq!(session.admitted_bundle, "grok-pty");
    assert_eq!(session.admitted_transport, "pty");
    assert_eq!(session.endpoint_provenance, "launch");
}

#[test]
fn diagnostic_claim_update_does_not_touch_admitted_facts() {
    let store = Store::open_memory().unwrap();
    let registration = reg("codex", "pk", "room");
    store
        .reserve_session_with_facts(
            &registration,
            &facts("codex", "", "codex-app", "acp", "launch"),
        )
        .unwrap();
    store.record_claimed_harness("pk", "claude-code").unwrap();

    let session = store.get_session("pk").unwrap().unwrap();
    assert_eq!(session.claimed_harness, "claude-code");
    assert_eq!(session.observed_harness, "codex");
    assert_eq!(session.admitted_bundle, "codex-app");
    assert_eq!(session.admitted_transport, "acp");
    assert_eq!(session.endpoint_provenance, "launch");
}
