use super::*;

#[test]
fn parses_real_tenex_shape_with_camelcase() {
    let json = r#"{
        "version": 3,
        "whitelistedPubkeys": ["aa", "bb"],
        "backendName": "pablos' laptop",
        "mosaicoPrivateKey": "deadbeef"
    }"#;
    let c = Config::from_json_str(json, "fallback").unwrap();
    assert_eq!(c.whitelisted_pubkeys, vec!["aa", "bb"]);
    assert_eq!(c.host, "pablos' laptop");
    assert!(c.relays.is_empty());
    assert_eq!(c.indexer_relay, DEFAULT_INDEXER_RELAY);
    assert_eq!(c.mosaico_private_key.as_deref(), Some("deadbeef"));
    assert_eq!(c.session_ikm_nsec().map(String::as_str), Some("deadbeef"));
    assert_eq!(c.management_nsec().map(String::as_str), Some("deadbeef"));
    assert_eq!(c.backend_nsec().map(String::as_str), Some("deadbeef"));
    assert!(c.user_nsec().is_none());
}

#[test]
fn key_accessors_split_when_both_present() {
    let json = r#"{
        "whitelistedPubkeys": [],
        "userNsec": "operatorkey",
        "mosaicoPrivateKey": "backendkey"
    }"#;
    let c = Config::from_json_str(json, "host").unwrap();
    assert_eq!(c.session_ikm_nsec().map(String::as_str), Some("backendkey"));
    assert_eq!(c.management_nsec().map(String::as_str), Some("backendkey"));
    assert_eq!(c.backend_nsec().map(String::as_str), Some("backendkey"));
    assert_eq!(c.user_nsec().map(String::as_str), Some("operatorkey"));
}

#[test]
fn user_nsec_alone_is_not_a_management_key() {
    let json = r#"{ "userNsec": "operatorkey" }"#;
    let c = Config::from_json_str(json, "host").unwrap();
    assert!(c.management_nsec().is_none());
    assert!(c.session_ikm_nsec().is_none());
    assert!(c.backend_nsec().is_none());
    assert_eq!(c.user_nsec().map(String::as_str), Some("operatorkey"));
}

#[test]
fn explicit_relays_win_and_host_falls_back() {
    let json = r#"{"whitelistedPubkeys":[],"relays":["wss://r1","wss://r2"]}"#;
    let c = Config::from_json_str(json, "fallback-host").unwrap();
    assert_eq!(c.relays, vec!["wss://r1", "wss://r2"]);
    assert_eq!(c.host, "fallback-host");
    assert!(c.whitelisted_pubkeys.is_empty());
    assert_eq!(c.indexer_relay, DEFAULT_INDEXER_RELAY);
}

#[test]
fn runtime_config_requires_an_explicit_relay() {
    let config = Config::from_json_str(r#"{"whitelistedPubkeys":[]}"#, "host").unwrap();
    let error = require_configured_relay(config).unwrap_err().to_string();

    assert!(error.contains("externally operated NIP-29 relay"));
    assert!(error.contains("mosaico setup --relay <ws-url>"));
}

#[test]
fn custom_indexer_relay() {
    let json = r#"{"indexerRelay":"wss://my-indexer.example"}"#;
    let c = Config::from_json_str(json, "host").unwrap();
    assert_eq!(c.indexer_relay, "wss://my-indexer.example");
}

#[test]
fn per_session_rooms_defaults_off_and_parses_when_set() {
    let off = Config::from_json_str(r#"{"whitelistedPubkeys":[]}"#, "host").unwrap();
    assert!(!off.per_session_rooms);
    let on = Config::from_json_str(
        r#"{"whitelistedPubkeys":[],"perSessionRooms":true}"#,
        "host",
    )
    .unwrap();
    assert!(on.per_session_rooms);
}

#[test]
fn attachment_directory_defaults_under_mosaico_home_and_resolves_relative_values() {
    let home = Path::new("/home/alice/.mosaico");
    assert_eq!(
        attachment_directory::resolve(None, home).unwrap(),
        home.join("tmp/attachments")
    );
    assert_eq!(
        attachment_directory::resolve(Some("shared/files".into()), home).unwrap(),
        home.join("shared/files")
    );
    assert_eq!(
        attachment_directory::resolve(Some("/var/tmp/mosaico".into()), home).unwrap(),
        PathBuf::from("/var/tmp/mosaico")
    );
}

#[test]
fn attachment_directory_absolutizes_relative_home_and_configured_value() {
    let current = std::env::current_dir().unwrap();
    assert_eq!(
        attachment_directory::resolve(
            Some("shared/../received".into()),
            Path::new("relative-mosaico-home"),
        )
        .unwrap(),
        current.join("relative-mosaico-home/received")
    );
    assert_eq!(
        attachment_directory::resolve(None, Path::new("relative-mosaico-home")).unwrap(),
        current.join("relative-mosaico-home/tmp/attachments")
    );
}

#[test]
fn daemon_upsert_persists_absolute_attachment_directory_and_unknown_fields() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("mosaico-home");
    std::fs::create_dir_all(&home).unwrap();
    let path = home.join("config.json");
    std::fs::write(
        &path,
        r#"{"relays":["wss://relay.example"],"unknown":{"keep":true}}"#,
    )
    .unwrap();
    let mut env = crate::test_env::EnvGuard::set("MOSAICO_HOME", &home);
    env.set_var("MOSAICO_CONFIG", &path);

    let directory = ensure_attachment_receive_directory().unwrap();
    let first = std::fs::read_to_string(&path).unwrap();
    ensure_attachment_receive_directory().unwrap();
    let second = std::fs::read_to_string(&path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&second).unwrap();

    assert_eq!(directory, home.join("tmp/attachments"));
    assert_eq!(
        json["attachmentReceiveDirectory"],
        directory.display().to_string()
    );
    assert_eq!(json["unknown"]["keep"], true);
    assert_eq!(first, second);
}

#[test]
fn cross_project_boundary_defaults_to_warn_reads_and_deny_writes() {
    let config = Config::from_json_str(r#"{"relays":["wss://relay.example"]}"#, "host").unwrap();

    assert_eq!(
        config.cross_project_boundary,
        CrossProjectBoundary {
            read: BoundaryAction::Warn,
            write: BoundaryAction::Deny,
        }
    );
}

#[test]
fn cross_project_boundary_accepts_every_configured_action() {
    for (value, expected) in [
        ("allow", BoundaryAction::Allow),
        ("warn", BoundaryAction::Warn),
        ("deny", BoundaryAction::Deny),
    ] {
        let body = format!(
            r#"{{
              "relays":["wss://relay.example"],
              "agents":{{"behavior":{{"crossProjectBoundary":{{
                "read":"{value}",
                "write":"{value}"
              }}}}}}
            }}"#
        );
        let config = Config::from_json_str(&body, "host").unwrap();
        assert_eq!(config.cross_project_boundary.read, expected);
        assert_eq!(config.cross_project_boundary.write, expected);
    }
}

#[test]
fn cross_project_boundary_rejects_unknown_actions() {
    let error = Config::from_json_str(
        r#"{
          "relays":["wss://relay.example"],
          "agents":{"behavior":{"crossProjectBoundary":{"read":"prompt"}}}
        }"#,
        "host",
    )
    .unwrap_err();

    assert!(error.to_string().contains("parsing mosaico config json"));
}

#[test]
fn mosaico_home_selection_uses_default_home_without_override() {
    let selected = select_mosaico_home(None, Some(OsString::from("/home/alice"))).unwrap();
    assert_eq!(selected.mosaico_home, PathBuf::from("/home/alice/.mosaico"));
    assert_eq!(
        selected.default_mosaico_home,
        Some(PathBuf::from("/home/alice/.mosaico"))
    );
    assert!(!selected.mosaico_home_set);
    assert!(selected.mosaico_home_is_default);
}

#[test]
fn config_path_defaults_inside_mosaico_home() {
    assert_eq!(
        select_config_path(None, PathBuf::from("/home/alice/.mosaico")),
        PathBuf::from("/home/alice/.mosaico/config.json")
    );
}

#[test]
fn config_path_honors_mosaico_config_override() {
    assert_eq!(
        select_config_path(
            Some(OsString::from("/tmp/custom-config.json")),
            PathBuf::from("/home/alice/.mosaico"),
        ),
        PathBuf::from("/tmp/custom-config.json")
    );
}

#[test]
fn mosaico_home_selection_reports_explicit_non_default_home() {
    let selected = select_mosaico_home(
        Some(OsString::from("/tmp/mosaico-test-home")),
        Some(OsString::from("/home/alice")),
    )
    .unwrap();
    assert_eq!(
        selected.mosaico_home,
        PathBuf::from("/tmp/mosaico-test-home")
    );
    assert_eq!(
        selected.default_mosaico_home,
        Some(PathBuf::from("/home/alice/.mosaico"))
    );
    assert!(selected.mosaico_home_set);
    assert!(!selected.mosaico_home_is_default);
}

#[test]
fn mosaico_home_selection_refuses_absent_home_without_override() {
    assert_eq!(select_mosaico_home(None, None), Err(MISSING_HOME_MESSAGE));
}
