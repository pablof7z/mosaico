use super::*;
use clap::{error::ErrorKind, Parser};

#[test]
fn top_level_doctor_flags_parse() {
    let cli = crate::cli::args::Cli::try_parse_from(["mosaico", "doctor", "--json"]).unwrap();
    match cli.cmd.expect("doctor command") {
        crate::cli::args::Cmd::Doctor(args) => {
            assert!(args.json);
            assert!(!args.fix);
        }
        _ => panic!("expected doctor command"),
    }
}

#[test]
fn fix_and_json_parse_as_one_agent_repair_invocation() {
    let cli =
        crate::cli::args::Cli::try_parse_from(["mosaico", "doctor", "--fix", "--json"]).unwrap();
    match cli.cmd.expect("doctor command") {
        crate::cli::args::Cmd::Doctor(args) => {
            assert!(args.json);
            assert!(args.fix);
        }
        _ => panic!("expected doctor command"),
    }
}

#[test]
fn removed_debug_doctor_spelling_is_rejected() {
    let error = crate::cli::args::Cli::try_parse_from(["mosaico", "debug", "doctor"])
        .err()
        .expect("old command must be gone");
    assert_eq!(error.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn valid_readback_requires_at_least_one_event() {
    assert!(readback_healthy("1 event(s) with #t=probe"));
    assert!(!readback_healthy("0 event(s) with #t=probe"));
    assert!(!readback_healthy("ERR relay unavailable"));
}

#[test]
fn skipped_probe_is_an_onboarding_warning_without_repair() {
    let check = relay_check(
        "relay.publish",
        "SKIP no authorized materialized group",
        false,
        "repair relay",
    );
    assert_eq!(check.status, CheckStatus::Warning);
    assert!(check.repair.is_none());
}

#[test]
fn invalid_config_is_actionable_without_exposing_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    std::fs::write(&path, "not-json").unwrap();
    let mut checks = Vec::new();

    assert!(!config::inspect(&path, &mut checks));
    assert_eq!(checks[0].name, "config.document");
    assert_eq!(checks[0].status, CheckStatus::Error);
    assert!(checks[0].repair.is_some());
    assert!(!checks[0].summary.contains("mosaicoPrivateKey"));
}

#[test]
fn missing_relay_is_an_error_with_explicit_setup_choices() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "mosaicoPrivateKey": crate::config::generate_mosaico_private_key(),
            "relays": [],
        })
        .to_string(),
    )
    .unwrap();
    let mut checks = Vec::new();

    assert!(!config::inspect(&path, &mut checks));
    let relay = checks
        .iter()
        .find(|check| check.name == "config.relays")
        .unwrap();
    assert_eq!(relay.status, CheckStatus::Error);
    assert!(relay
        .repair
        .as_deref()
        .unwrap()
        .contains("externally operated NIP-29 relay"));
}

#[test]
fn human_report_shows_repairs_and_verdict() {
    let report = DoctorReport {
        healthy: false,
        fix_attempted: false,
        storage: serde_json::json!({"mosaico_home": "/tmp/mosaico"}),
        repairs: Vec::new(),
        checks: vec![
            Check::new("skill", CheckStatus::Error, "missing").repair("run `mosaico doctor --fix`")
        ],
    };
    let rendered = render::human(&report);
    assert!(rendered.contains("mosaico doctor: needs attention"));
    assert!(rendered.contains("[error] skill: missing"));
    assert!(rendered.contains("mosaico doctor --fix"));
}
