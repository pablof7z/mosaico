use super::{extract_agent_flag, harness_from_process};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn extract_agent_flag_finds_space_separated_form() {
    let a = argv(&["claude", "--agent", "chief-of-staff"]);
    assert_eq!(extract_agent_flag(&a).as_deref(), Some("chief-of-staff"));
}

#[test]
fn extract_agent_flag_finds_equals_form() {
    let a = argv(&["claude", "--agent=chief-of-staff"]);
    assert_eq!(extract_agent_flag(&a).as_deref(), Some("chief-of-staff"));
}

#[test]
fn extract_agent_flag_ignores_agents_flag() {
    let a = argv(&["claude", "--agents", r#"{"x":1}"#]);
    assert_eq!(extract_agent_flag(&a), None);
}

#[test]
fn detects_hermes_python_entrypoint_without_matching_hook_arguments() {
    assert_eq!(
        harness_from_process(
            "/opt/hermes/bin/python3",
            "/opt/hermes/bin/python3 /opt/hermes/bin/hermes acp"
        ),
        Some("hermes")
    );
    assert_eq!(
        harness_from_process("mosaico", "mosaico harness hook hermes --type stop"),
        None
    );
}

#[test]
fn detects_pi_node_entrypoint_without_matching_hook_arguments() {
    assert_eq!(
        harness_from_process(
            "/usr/local/bin/node",
            "/usr/local/bin/node /opt/node_modules/@earendil-works/pi-coding-agent/dist/cli.js --mode rpc"
        ),
        Some("pi")
    );
    assert_eq!(harness_from_process("mosaico", "harness hook pi"), None);
}

#[test]
fn detects_only_the_exact_pi_executable_basename() {
    assert_eq!(harness_from_process("pi", "pi"), Some("pi"));
    assert_eq!(
        harness_from_process("/usr/local/bin/pi", "/usr/local/bin/pi"),
        Some("pi")
    );
    assert_eq!(harness_from_process("pipeline", "pipeline"), None);
    assert_eq!(harness_from_process("python", "python script.py"), None);
}

#[test]
fn extract_agent_flag_absent_when_no_flag() {
    let a = argv(&["claude", "--dangerously-skip-permissions"]);
    assert_eq!(extract_agent_flag(&a), None);
}

#[test]
fn extract_agent_flag_dangling_flag_at_end_yields_none() {
    let a = argv(&["claude", "--agent"]);
    assert_eq!(extract_agent_flag(&a), None);
}
