use super::*;

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
    };

    let error = preflight_selection(&selection).unwrap_err().to_string();

    assert!(error.contains("hooks must be a JSON object"));
}
