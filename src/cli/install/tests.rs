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

fn write_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::write(path, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn installer_lists_hooked_harnesses_while_goose_remains_native() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for executable in ["claude", "codex", "opencode", "grok", "goose", "hermes"] {
        write_executable(&bin.join(executable));
    }
    let mut env = crate::test_env::EnvGuard::set("HOME", temp.path());
    env.set_var("PATH", &bin);

    let all = harnesses().unwrap();
    assert_eq!(
        all.iter().map(|harness| harness.id).collect::<Vec<_>>(),
        ["claude-code", "codex", "opencode", "grok", "hermes"]
    );
    assert!(all.iter().all(|harness| harness.detected));
    assert!(crate::config::detect_available_harnesses()
        .unwrap()
        .contains(&crate::session::Harness::Goose));

    let selection = resolve_selection(&all, &opts(true, None)).unwrap();
    assert!(selection.skill, "Goose consumes the canonical agents skill");
    assert_eq!(selection.harnesses.len(), 5);
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
    let options = InstallOpts::uninstall(false);

    let selection = resolve_selection(&harnesses, &options).unwrap();

    assert!(selection.skill);
    assert_eq!(selection.harnesses.len(), 2);
}

#[test]
fn merge_hooks_preserves_foreign_groups_and_replaces_ours() {
    let mut root = serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [
                {
                    "hooks": [{
                        "type": "command",
                        "command": "pc hook inject --harness codex",
                        "timeout": 30
                    }]
                },
                {
                    "hooks": [{
                        "type": "command",
                        "command": "mosaico harness hook codex --type old",
                        "timeout": 1
                    }]
                }
            ]
        }
    });

    merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);

    let groups = root
        .pointer("/hooks/UserPromptSubmit")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().any(|g| {
        g.pointer("/hooks/0/command")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c == "pc hook inject --harness codex")
    }));
    assert!(groups.iter().any(|g| {
        g.pointer("/hooks/0/command")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c == "mosaico harness hook codex --type user-prompt-submit")
    }));
}

#[test]
fn uninstall_removes_ours_and_empty_events_only() {
    let mut root = serde_json::json!({
        "hooks": {
            "Stop": [
                {
                    "hooks": [{
                        "type": "command",
                        "command": "mosaico harness hook codex --type stop",
                        "timeout": 30
                    }]
                }
            ],
            "UserPromptSubmit": [
                {
                    "hooks": [{
                        "type": "command",
                        "command": "pc hook inject --harness codex",
                        "timeout": 30
                    }]
                },
                {
                    "hooks": [{
                        "type": "command",
                        "command": "mosaico harness hook codex --type user-prompt-submit",
                        "timeout": 30
                    }]
                }
            ]
        }
    });

    let removed = merge_hooks(&mut root, &config::codex_hook_entries(), "codex", true);

    assert_eq!(removed, 2);
    assert!(root.pointer("/hooks/Stop").is_none());
    let groups = root
        .pointer("/hooks/UserPromptSubmit")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0]
            .pointer("/hooks/0/command")
            .and_then(|v| v.as_str()),
        Some("pc hook inject --harness codex")
    );
}

#[test]
fn codex_root_events_are_migrated_under_hooks() {
    let mut root = serde_json::json!({
        "Stop": [{
            "hooks": [{
                "type": "command",
                "command": "foreign stop",
                "timeout": 1
            }]
        }],
        "hooks": {
            "Stop": [{
                "hooks": [{
                    "type": "command",
                    "command": "existing stop",
                    "timeout": 1
                }]
            }]
        }
    });

    migrate_codex_root_events(&mut root);

    assert!(root.get("Stop").is_none());
    let groups = root
        .pointer("/hooks/Stop")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(groups.len(), 2);
}

#[test]
fn write_json_creates_parent_directories() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("a/b/hooks.json");
    write_json(&path, &serde_json::json!({"hooks": {}})).unwrap();
    assert!(path.exists());
}

#[test]
fn status_detects_installed_codex_hooks() {
    let temp = tempfile::tempdir().unwrap();
    let h = harness("codex", temp.path().join("hooks.json"));
    let mut root = serde_json::json!({});
    merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);
    write_json(&h.config_path, &root).unwrap();

    assert!(is_installed(&h));
    assert!(is_present(&h));
}

#[test]
fn detected_harness_without_mosaico_trace_is_not_selected_for_repair() {
    let temp = tempfile::tempdir().unwrap();
    let h = harness("codex", temp.path().join("hooks.json"));
    write_json(
        &h.config_path,
        &serde_json::json!({"hooks": {"Stop": [{"hooks": [{"command": "foreign"}]}]}}),
    )
    .unwrap();

    assert!(!is_present(&h));
    assert!(!is_installed(&h));
}

#[test]
fn partial_mosaico_hooks_are_selected_for_repair() {
    let temp = tempfile::tempdir().unwrap();
    let h = harness("codex", temp.path().join("hooks.json"));
    write_json(
        &h.config_path,
        &serde_json::json!({
            "hooks": {"Stop": [{"hooks": [{"command": "mosaico harness hook codex --type stop"}]}]}
        }),
    )
    .unwrap();

    assert!(is_present(&h));
    assert!(!is_installed(&h));
}

#[test]
fn installation_requires_at_least_one_wired_harness() {
    let temp = tempfile::tempdir().unwrap();
    let codex = harness("codex", temp.path().join("hooks.json"));
    let opencode = harness("opencode", temp.path().join("mosaico.ts"));

    assert!(![&codex, &opencode].into_iter().any(is_installed));

    let mut root = serde_json::json!({});
    merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);
    write_json(&codex.config_path, &root).unwrap();

    assert!([&codex, &opencode].into_iter().any(is_installed));
}
