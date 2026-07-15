use super::*;

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
}

#[test]
fn named_profile_deep_merges_and_materializes_an_overlay_home() {
    let source = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    write(
        &source.path().join("config.toml"),
        r#"
model = "base"
[tools]
web_search = false
[mcp_servers.base]
command = "base-mcp"
"#,
    );
    write(
        &source.path().join("planner.config.toml"),
        r#"
model = "planner"
[tools]
web_search = true
[mcp_servers.planner]
command = "planner-mcp"
"#,
    );
    write(&source.path().join("auth.json"), "{}");
    std::fs::create_dir(source.path().join("sessions")).unwrap();

    let plan = plan("planner", source.path(), scratch.path()).unwrap();
    plan.materialize().unwrap();

    let target = scratch.path().join("codex-home");
    let merged: toml::Value =
        toml::from_str(&std::fs::read_to_string(target.join("config.toml")).unwrap()).unwrap();
    assert_eq!(merged["model"].as_str(), Some("planner"));
    assert_eq!(merged["tools"]["web_search"].as_bool(), Some(true));
    assert_eq!(
        merged["mcp_servers"]["base"]["command"].as_str(),
        Some("base-mcp")
    );
    assert_eq!(
        merged["mcp_servers"]["planner"]["command"].as_str(),
        Some("planner-mcp")
    );
    assert!(std::fs::symlink_metadata(target.join("auth.json"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(std::fs::symlink_metadata(target.join("sessions"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        plan.extra_env,
        vec![(
            "CODEX_HOME".to_string(),
            target.to_string_lossy().into_owned()
        )]
    );
}

#[test]
fn profile_name_validation_rejects_paths_and_whitespace() {
    let source = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    for invalid in ["", "../planner", "planner profile", "planner.toml"] {
        let error = plan(invalid, source.path(), scratch.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid Codex profile"), "{error}");
    }
}

#[test]
fn profile_name_validation_accepts_current_codex_names() {
    for valid in ["planner", "deep-review", "planner_v2", "P3"] {
        validate_name(valid).unwrap();
    }
}
