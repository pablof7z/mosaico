use super::*;

fn facts(
    observed: &str,
    claimed: &str,
    preset: &str,
    transport: &str,
    provenance: &str,
) -> AdmittedRuntimeFacts {
    AdmittedRuntimeFacts {
        observed_harness: observed.into(),
        claimed_harness: claimed.into(),
        preset: preset.into(),
        transport: transport.into(),
        endpoint_provenance: provenance.into(),
    }
}

#[test]
fn hook_claim_cannot_rewrite_launch_facts_even_after_the_row_is_dead() {
    let store = Store::open_memory().unwrap();
    let registration = reg("grok", "pk", "room");
    let generation = store
        .reserve_session_with_facts(
            &registration,
            &facts("grok", "", "unrestricted", "pty", "launch"),
        )
        .unwrap();
    assert!(store
        .mark_runtime_stopped_if_generation("pk", generation, StopReason::Unknown, 1_500)
        .unwrap());

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
    assert_eq!(session.admitted_preset, "unrestricted");
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
            &facts("codex", "", "", "app-server", "launch"),
        )
        .unwrap();
    store.record_claimed_harness("pk", "claude-code").unwrap();

    let session = store.get_session("pk").unwrap().unwrap();
    assert_eq!(session.claimed_harness, "claude-code");
    assert_eq!(session.observed_harness, "codex");
    assert_eq!(session.admitted_preset, "");
    assert_eq!(session.admitted_transport, "app-server");
    assert_eq!(session.endpoint_provenance, "launch");
}

#[test]
fn store_rejects_incomplete_or_inconsistent_runtime_facts() {
    let cases = [
        (
            facts("", "", "unrestricted", "pty", "launch"),
            "require observed_harness",
        ),
        (
            facts("grok", "", "unrestricted", "pty", "launch"),
            "does not match admitted facts",
        ),
        (
            facts("codex", "codex", "unrestricted", "pty", "launch"),
            "launch runtime facts forbid claimed_harness",
        ),
        (
            facts("codex", "", "unrestricted", "", "launch"),
            "launch runtime facts require transport",
        ),
        (
            facts("codex", "", "", "", "hook"),
            "hook runtime facts require claimed_harness",
        ),
        (
            facts("codex", "codex", "unrestricted", "pty", "hook"),
            "hook runtime facts forbid preset",
        ),
        (
            facts("codex", "codex", "", "exec", "hook"),
            "unknown transport",
        ),
        (
            facts("codex", "codex", "", "", "migration"),
            "endpoint_provenance launch or hook",
        ),
    ];

    for (candidate, expected) in cases {
        let store = Store::open_memory().unwrap();
        let error = store
            .reserve_session_with_facts(&reg("codex", "pk", "room"), &candidate)
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
        assert!(store.get_session("pk").unwrap().is_none());
    }
}
