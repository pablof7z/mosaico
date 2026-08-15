use super::*;

#[test]
fn pi_installation_requires_the_current_owned_extension() {
    let temp = tempfile::tempdir().unwrap();
    let h = harness("pi", temp.path().join("mosaico"));
    write_text(
        &h.config_path.join("index.ts"),
        "export default function stale() {}\n",
    )
    .unwrap();
    assert!(!is_installed(&h));

    write_text(&temp.path().join("mosaico.ts"), PI_EXTENSION_TS).unwrap();
    write_text(&temp.path().join("tools.ts"), PI_TOOLS_TS).unwrap();
    pi::install(&h, &InstallOpts::default(), false).unwrap();
    assert!(is_installed(&h));
    for (name, source) in PI_EXTENSION_FILES {
        assert_eq!(
            std::fs::read_to_string(h.config_path.join(name)).unwrap(),
            *source
        );
    }
    assert!(!temp.path().join("mosaico.ts").exists());
    assert!(!temp.path().join("tools.ts").exists());

    pi::install(
        &h,
        &InstallOpts::uninstall(Some("pi".to_string()), false),
        false,
    )
    .unwrap();
    assert!(!h.config_path.exists());
}

#[test]
fn pi_installation_evicts_the_legacy_npm_package_so_tools_do_not_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let agent_dir = temp.path().join(".pi/agent");
    let h = harness("pi", agent_dir.join("extensions/mosaico"));
    let settings = agent_dir.join("settings.json");
    write_text(
        &settings,
        r#"{"extensions":["npm:pi-web-access","npm:pi-mosaico","npm:pi-memory-stone"]}"#,
    )
    .unwrap();
    let npm_pkg = agent_dir.join("npm/node_modules/pi-mosaico");
    std::fs::create_dir_all(&npm_pkg).unwrap();
    write_text(&npm_pkg.join("mosaico.ts"), "// stale single-file form\n").unwrap();

    pi::install(&h, &InstallOpts::default(), false).unwrap();
    assert!(is_installed(&h));

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    let kept: Vec<String> = doc
        .get("extensions")
        .and_then(|e| e.as_array())
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    assert!(
        !kept.iter().any(|s| s == "npm:pi-mosaico"),
        "npm:pi-mosaico still listed"
    );
    assert!(
        kept.contains(&"npm:pi-web-access".to_string()),
        "unrelated extension evicted"
    );
    assert!(!npm_pkg.exists(), "stale npm package dir still present");
}
