use super::*;

#[test]
fn preserves_valid_slug() {
    assert_eq!(persistable_slug("reviewer"), "reviewer");
}

#[test]
fn sanitizes_native_profile_name() {
    assert_eq!(persistable_slug("Ava Chen"), "ava-chen");
}

#[test]
fn canonical_harness_choices_include_pi() {
    assert!(Harness::ALL.contains(&Harness::Pi));
}

#[test]
fn native_profile_is_not_saved_as_generic_profile() {
    let row = AgentRow {
        slug: "reviewer".into(),
        agent_slug: "reviewer".into(),
        description: String::new(),
        harness: Harness::ClaudeCode,
        profile: None,
        preset: None,
        per_session_key: None,
        kind: AgentKind::NativeProfile,
        native_profile: None,
    };
    assert_eq!(profile_for_save(&row), None);
}
