use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn command(name: &str, parts: &[&str]) -> LaunchCommand {
    LaunchCommand::new(name, argv(parts)).unwrap()
}

#[test]
fn suggestions_adapt_other_agent_commands() {
    let agents = vec![(
        "poppy".to_string(),
        vec![command(
            "file",
            &["runner", "--file", "/home/me/poppy.json"],
        )],
        None,
        None,
    )];

    let suggestions = agent_command_suggestions("newagent", &agents);
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].command.name, "file");
    assert_eq!(
        suggestions[0].command.argv,
        argv(&["runner", "--file", "/home/me/newagent.json"])
    );
}

#[test]
fn suggestions_ignore_old_singular_command_shape() {
    let agents = vec![("legacy".to_string(), vec![], None, None)];
    assert!(agent_command_suggestions("newagent", &agents).is_empty());
}

#[test]
fn builtins_prefer_matching_target_slug() {
    let suggestions = builtin_command_suggestions("codex");
    assert_eq!(suggestions[0].command.name, "codex");
    assert_eq!(suggestions[0].command.argv, argv(&["codex"]));
}

#[test]
fn duplicate_extra_args_are_not_appended_twice() {
    let base = argv(&["codex", "--yolo"]);
    let extra = argv(&["--yolo"]);
    assert!(extra_args_without_duplicate_suffix(&base, extra).is_empty());
}

#[test]
fn distinct_extra_args_are_preserved() {
    let base = argv(&["codex", "--model", "gpt-5"]);
    let extra = argv(&["--yolo"]);
    assert_eq!(
        extra_args_without_duplicate_suffix(&base, extra),
        argv(&["--yolo"])
    );
}

#[test]
fn suggestions_dedup_identical_argv_across_agents() {
    let agents = vec![
        (
            "chief-of-staff".to_string(),
            vec![command(
                "codex",
                &["codex", "--yolo", "--profile", "planner"],
            )],
            None,
            None,
        ),
        (
            "ios-tester".to_string(),
            vec![command(
                "codex",
                &["codex", "--yolo", "--profile", "planner"],
            )],
            None,
            None,
        ),
    ];

    let suggestions = agent_command_suggestions("planner", &agents);
    assert_eq!(suggestions.len(), 1);
    assert_eq!(
        suggestions[0].command.argv,
        argv(&["codex", "--yolo", "--profile", "planner"])
    );
    assert_eq!(suggestions[0].label, "codex --yolo --profile planner");
}

#[test]
fn suggestions_keep_binary_when_source_slug_matches_binary() {
    // Agent "codex" configured with `codex --yolo` must NOT have its
    // binary rewritten to the target slug when adapting for "planner".
    let agents = vec![(
        "codex".to_string(),
        vec![command("yolo", &["codex", "--yolo"])],
        None,
        None,
    )];

    let suggestions = agent_command_suggestions("planner", &agents);
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].command.argv, argv(&["codex", "--yolo"]));
    assert_eq!(suggestions[0].label, "codex --yolo");
}

#[test]
fn display_argv_quotes_shell_sensitive_args() {
    assert_eq!(
        display_argv(&argv(&["codex", "--profile", "work profile"])),
        "codex --profile 'work profile'"
    );
}
