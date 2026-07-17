use super::*;

fn bundle(harness: Harness, transport: Transport, args: &[&str]) -> HarnessBundle {
    HarnessBundle {
        harness,
        transport,
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
    }
}

#[test]
fn ensure_bundle_reuses_an_exact_existing_entry() {
    let mut config = HarnessesConfig::default();
    config.bundles.insert(
        "my-claude".into(),
        bundle(Harness::ClaudeCode, Transport::Acp, &[]),
    );

    let (name, created) = config
        .ensure_bundle(
            "claude-acp",
            bundle(Harness::ClaudeCode, Transport::Acp, &[]),
        )
        .unwrap();

    assert_eq!(name, "my-claude");
    assert!(!created);
    assert_eq!(config.bundles.len(), 1);
}

#[test]
fn ensure_bundle_preserves_conflicts_and_uses_stable_suffixes() {
    let mut config = HarnessesConfig::default();
    let tuned = bundle(
        Harness::ClaudeCode,
        Transport::Pty,
        &["--dangerously-skip-permissions"],
    );
    config.bundles.insert("claude-pty".into(), tuned.clone());
    config.bundles.insert(
        "claude-pty-2".into(),
        bundle(Harness::Codex, Transport::Pty, &[]),
    );

    let desired = bundle(Harness::ClaudeCode, Transport::Pty, &[]);
    let (name, created) = config.ensure_bundle("claude-pty", desired.clone()).unwrap();

    assert!(created);
    assert_eq!(name, "claude-pty-3");
    assert_eq!(config.bundles["claude-pty"], tuned);
    assert_eq!(config.bundles["claude-pty-3"], desired);
}

#[test]
fn save_round_trip_preserves_every_entry_and_arg() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/harnesses.json");
    let mut config = HarnessesConfig::default();
    config.bundles.insert(
        "claude-pty".into(),
        bundle(
            Harness::ClaudeCode,
            Transport::Pty,
            &["--dangerously-skip-permissions"],
        ),
    );
    config.bundles.insert(
        "codex-app".into(),
        bundle(Harness::Codex, Transport::AppServer, &[]),
    );

    config.save_to(&path).unwrap();

    assert_eq!(HarnessesConfig::load_from(&path).unwrap(), config);
    assert!(std::fs::read_to_string(path).unwrap().ends_with('\n'));
}

#[test]
fn ensure_bundle_rejects_a_blank_name() {
    let mut config = HarnessesConfig::default();
    assert!(config
        .ensure_bundle("  ", bundle(Harness::Codex, Transport::Pty, &[]))
        .is_err());
}

#[test]
fn hosted_resolution_uses_one_tuned_bundle_or_creates_a_default() {
    let mut configured = HarnessesConfig::default();
    configured.bundles.insert(
        "my-codex".into(),
        bundle(Harness::Codex, Transport::Pty, &["--yolo"]),
    );
    assert_eq!(
        configured
            .resolve_or_create_hosted(Harness::Codex, Transport::Pty)
            .unwrap(),
        ("my-codex".into(), false)
    );

    let mut empty = HarnessesConfig::default();
    assert_eq!(
        empty
            .resolve_or_create_hosted(Harness::ClaudeCode, Transport::Pty)
            .unwrap(),
        ("claude-pty".into(), true)
    );
    assert_eq!(
        empty.get("claude-pty"),
        Some(&bundle(Harness::ClaudeCode, Transport::Pty, &[]))
    );
}

#[test]
fn hosted_resolution_rejects_multiple_policy_candidates() {
    let mut config: HarnessesConfig = serde_json::from_str(
        r#"{
          "codex-safe":{"harness":"codex","transport":"pty"},
          "codex-yolo":{"harness":"codex","transport":"pty","args":["--yolo"]}
        }"#,
    )
    .unwrap();

    let error = config
        .resolve_or_create_hosted(Harness::Codex, Transport::Pty)
        .unwrap_err();
    assert!(error.to_string().contains("multiple codex pty bundles"));
}
