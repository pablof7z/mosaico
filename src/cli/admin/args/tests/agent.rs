use super::*;

#[test]
fn agent_add_requires_bundle_and_accepts_profile_and_workspace() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "mgmt",
        "agent",
        "add",
        "reviewer",
        "--harness",
        "yolo-claude",
        "--profile",
        "reviewer",
        "--workspace",
        "mosaico",
    ])
    .unwrap();

    let crate::cli::args::Cmd::Mgmt {
        action:
            crate::cli::args::MgmtAction::Agent {
                action:
                    AgentAction::Add {
                        slug,
                        harness,
                        profile,
                        workspaces,
                    },
            },
    } = cli.cmd
    else {
        panic!("expected mgmt agent add command");
    };
    assert_eq!(slug, "reviewer");
    assert_eq!(harness, "yolo-claude");
    assert_eq!(profile.as_deref(), Some("reviewer"));
    assert_eq!(workspaces, ["mosaico"]);
}
