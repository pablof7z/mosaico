use super::*;

fn harness(id: &'static str, path: std::path::PathBuf) -> Harness {
    Harness {
        id,
        display: id,
        config_path: path,
        detected: true,
    }
}

fn opts(all: bool, harness: Option<&str>) -> InstallOpts {
    InstallOpts {
        all,
        harness: harness.map(str::to_string),
        ..InstallOpts::default()
    }
}

#[test]
fn malformed_selected_harness_is_rejected_before_writes() {
    let temp = tempfile::tempdir().unwrap();
    let harness = Harness {
        id: "codex",
        display: "codex",
        config_path: temp.path().join("hooks.json"),
        detected: true,
    };
    std::fs::write(&harness.config_path, r#"{"hooks": []}"#).unwrap();
    let selection = InstallSelection {
        skill: true,
        harnesses: vec![&harness],
        wrappers: None,
    };

    let error = preflight_selection(&selection).unwrap_err().to_string();

    assert!(error.contains("hooks must be a JSON object"));
}

#[test]
fn all_selection_includes_skill_and_detected_harnesses() {
    let temp = tempfile::tempdir().unwrap();
    let harnesses = vec![
        harness("codex", temp.path().join("codex.json")),
        Harness {
            detected: false,
            ..harness("opencode", temp.path().join("opencode.ts"))
        },
    ];

    let selection = resolve_selection(&harnesses, &opts(true, None)).unwrap();

    assert!(selection.skill);
    assert_eq!(selection.harnesses.len(), 1);
    assert_eq!(selection.harnesses[0].id, "codex");
}

#[test]
fn explicit_harness_selection_includes_skill() {
    let temp = tempfile::tempdir().unwrap();
    let harnesses = vec![harness("codex", temp.path().join("codex.json"))];

    let selection = resolve_selection(&harnesses, &opts(false, Some("codex"))).unwrap();

    assert!(selection.skill);
    assert_eq!(selection.harnesses.len(), 1);
    assert_eq!(selection.harnesses[0].id, "codex");
}

#[test]
fn unknown_harness_ids_fail_before_any_write() {
    let temp = tempfile::tempdir().unwrap();
    let harnesses = vec![harness("codex", temp.path().join("codex.json"))];

    let error = resolve_selection(&harnesses, &opts(false, Some("nope")))
        .err()
        .expect("unknown harness id must fail")
        .to_string();
    assert!(error.contains("unknown harness id(s): nope"));

    let options = InstallOpts::uninstall(Some("nope".into()), false);
    let error = resolve_selection(&harnesses, &options)
        .err()
        .expect("unknown harness id must fail")
        .to_string();
    assert!(error.contains("unknown harness id(s): nope"));
}

#[test]
fn uninstall_selection_includes_every_harness_even_when_not_detected() {
    let temp = tempfile::tempdir().unwrap();
    let harnesses = vec![
        Harness {
            detected: false,
            ..harness("codex", temp.path().join("codex.json"))
        },
        Harness {
            detected: false,
            ..harness("opencode", temp.path().join("opencode.ts"))
        },
    ];
    let options = InstallOpts::uninstall(None, false);

    let selection = resolve_selection(&harnesses, &options).unwrap();

    assert!(selection.skill);
    assert_eq!(selection.harnesses.len(), 2);
    assert!(selection.wrappers.is_none());
}

#[test]
fn scoped_uninstall_selects_only_one_harness_and_preserves_skill() {
    let temp = tempfile::tempdir().unwrap();
    let harnesses = vec![
        harness("codex", temp.path().join("codex.json")),
        harness("opencode", temp.path().join("opencode.ts")),
    ];
    let options = InstallOpts::uninstall(Some("codex".into()), false);

    let selection = resolve_selection(&harnesses, &options).unwrap();

    assert!(!selection.skill);
    assert_eq!(selection.harnesses.len(), 1);
    assert_eq!(selection.harnesses[0].id, "codex");
}

#[test]
fn explicit_wrapper_selection_is_exact_and_must_be_installed() {
    let temp = tempfile::tempdir().unwrap();
    let harnesses = vec![
        harness("claude-code", temp.path().join("claude.json")),
        harness("codex", temp.path().join("codex.json")),
    ];
    let mut options = opts(false, Some("codex"));
    options.wrap = Some("codex".into());

    let selection = resolve_selection(&harnesses, &options).unwrap();

    assert_eq!(selection.wrappers.unwrap()[0].id, "codex");

    options.wrap = Some("claude-code".into());
    let error = resolve_selection(&harnesses, &options)
        .err()
        .expect("wrapper outside setup selection must fail")
        .to_string();
    assert!(error.contains("must also be selected for setup"));
}
