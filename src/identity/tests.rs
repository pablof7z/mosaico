use super::*;

#[test]
fn creates_and_reloads_canonical_agent_config() {
    let dir = tempfile::tempdir().unwrap();
    let created = load_or_create(
        dir.path(),
        "coder",
        "codex",
        Some("reviewer"),
        Some("unrestricted"),
        100,
    )
    .unwrap();
    let loaded = load(dir.path(), "coder").unwrap();
    assert!(created.keys.is_none());
    assert_eq!(loaded.harness, "codex");
    assert_eq!(loaded.profile.as_deref(), Some("reviewer"));
    assert_eq!(loaded.preset.as_deref(), Some("unrestricted"));
}

#[test]
fn rejects_noncanonical_harness_names() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_or_create(dir.path(), "coder", "codex-pty", None, None, 1).is_err());
    assert!(load_or_create(dir.path(), "coder", "custom", None, None, 1).is_err());
}

#[test]
fn structured_save_changes_harness_and_preset() {
    let dir = tempfile::tempdir().unwrap();
    add_local_agent(dir.path(), "coder", "codex", None, None, 1).unwrap();
    let (saved, created) = save_local_agent(
        dir.path(),
        "coder",
        LocalAgentUpdate {
            harness: "claude-code".into(),
            profile: None,
            preset: Some("unrestricted".into()),
            per_session_key: Some(true),
            byline: Some(Some("Reviews changes".into())),
        },
        2,
    )
    .unwrap();
    assert!(!created);
    assert_eq!(saved.harness, "claude-code");
    assert_eq!(saved.preset.as_deref(), Some("unrestricted"));
}

#[test]
fn rejects_bad_slug() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_or_create(dir.path(), "bad slug", "codex", None, None, 1).is_err());
}

#[test]
fn remove_is_permanent() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", "codex", None, None, 1).unwrap();
    assert!(remove_local_agent(dir.path(), "coder").unwrap());
    assert!(!dir.path().join("agents/coder.json").exists());
    assert!(!remove_local_agent(dir.path(), "coder").unwrap());
}
