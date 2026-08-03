use super::*;

fn write_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let body = if path.file_name().and_then(|name| name.to_str()) == Some("goose") {
        "#!/bin/sh\necho 1.43.0\n"
    } else {
        "#!/bin/sh\n"
    };
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn installer_lists_every_supported_harness() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for executable in [
        "claude", "codex", "opencode", "grok", "goose", "hermes", "kimi",
    ] {
        write_executable(&bin.join(executable));
    }
    let mut env = crate::test_env::EnvGuard::set("HOME", temp.path());
    env.set_var("PATH", &bin);

    let all = harnesses().unwrap();
    assert_eq!(
        all.iter().map(|harness| harness.id).collect::<Vec<_>>(),
        [
            "claude-code",
            "codex",
            "opencode",
            "grok",
            "goose",
            "hermes",
            "kimi"
        ]
    );
    assert!(all.iter().all(|harness| harness.detected));
    assert!(crate::config::detect_available_harnesses()
        .unwrap()
        .contains(&crate::session::Harness::Goose));

    let selection = resolve_selection(&all, &opts(true, None)).unwrap();
    assert!(selection.skill);
    assert_eq!(selection.harnesses.len(), 7);
}
