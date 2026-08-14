use super::*;

#[test]
fn home_dir_uses_home_env() {
    assert_eq!(
        home_dir_from_env(Some("/Users/alice".to_string())).unwrap(),
        PathBuf::from("/Users/alice")
    );
}

#[test]
fn home_dir_refuses_absent_or_empty_home() {
    for home in [None, Some(String::new())] {
        let err = home_dir_from_env(home).unwrap_err().to_string();
        assert!(err.contains("HOME is not set"));
        assert!(err.contains("MOSAICO and MOSAICO_HOME only select"));
    }
}

#[test]
fn grok_home_honors_override_and_defaults_under_home() {
    let home = PathBuf::from("/Users/alice");
    assert_eq!(
        grok_home_dir(Some("/tmp/grok".to_string()), &home),
        PathBuf::from("/tmp/grok")
    );
    assert_eq!(grok_home_dir(None, &home), home.join(".grok"));
    assert_eq!(
        grok_home_dir(Some(String::new()), &home),
        home.join(".grok")
    );
}

#[test]
fn hermes_home_honors_override_and_defaults_under_home() {
    let home = PathBuf::from("/Users/alice");
    assert_eq!(
        hermes_home_dir(Some("/tmp/hermes".to_string()), &home),
        PathBuf::from("/tmp/hermes")
    );
    assert_eq!(hermes_home_dir(None, &home), home.join(".hermes"));
}

#[test]
fn kimi_home_honors_override_and_defaults_under_home() {
    let home = PathBuf::from("/Users/alice");
    assert_eq!(
        kimi_home_dir(Some("/tmp/kimi".to_string()), &home),
        PathBuf::from("/tmp/kimi")
    );
    assert_eq!(kimi_home_dir(None, &home), home.join(".kimi-code"));
}

#[test]
fn pi_agent_dir_honors_override_and_defaults_under_home() {
    let home = PathBuf::from("/Users/alice");
    assert_eq!(
        pi_agent_dir(Some("/tmp/pi-agent".to_string()), &home),
        PathBuf::from("/tmp/pi-agent")
    );
    assert_eq!(pi_agent_dir(None, &home), home.join(".pi/agent"));
    assert_eq!(
        pi_agent_dir(Some(String::new()), &home),
        home.join(".pi/agent")
    );
}
