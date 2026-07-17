use super::*;

#[test]
fn agents_command_parses_without_an_action() {
    let cli = Cli::try_parse_from(["mosaico", "agents"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Agents(_)));
}

#[test]
fn removed_mgmt_command_stays_unavailable() {
    assert_eq!(
        parse_err(&["mosaico", "mgmt", "agent", "list"]).kind(),
        ErrorKind::InvalidSubcommand
    );
}

#[test]
fn contextual_help_shows_agents_only_to_humans() {
    let agent_help = super::super::command_for_context(true)
        .render_long_help()
        .to_string();
    assert!(!agent_help.contains("  agents"));
    assert!(!agent_help.contains("  mgmt"));

    let human_help = super::super::command_for_context(false)
        .render_long_help()
        .to_string();
    assert!(human_help.contains("  agents"));
    assert!(!human_help.contains("  mgmt"));
}
