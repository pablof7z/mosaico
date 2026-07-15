use super::*;
use tempfile::TempDir;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn roots(home: &Path) -> DiscoveryRoots {
    DiscoveryRoots::for_user_home(home)
}

#[test]
fn discovers_supported_global_profiles_without_mosaico_agent_json() {
    let home = TempDir::new().unwrap();
    write(
        &home.path().join(".codex/agents/reviewer.toml"),
        r#"name = "reviewer"
description = "Reviews correctness"
developer_instructions = "Review like an owner"
"#,
    );
    write(
        &home.path().join(".claude/agents/designer.md"),
        "---\nname: designer\ndescription: Designs interfaces\n---\nPrompt",
    );
    write(
        &home.path().join(".config/opencode/agents/researcher.md"),
        "---\ndescription: Finds primary evidence\nmode: primary\n---\nPrompt",
    );

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    let capabilities = catalog.capabilities(None);
    assert_eq!(
        capabilities
            .iter()
            .map(|capability| capability.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["designer", "researcher", "reviewer"]
    );
    assert_eq!(
        catalog.resolve("reviewer", None, None).unwrap().harness,
        Harness::Codex
    );
}

#[test]
fn workspace_profile_overrides_global_profile_for_same_harness() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    write(
        &home.path().join(".codex/agents/reviewer.toml"),
        "name='reviewer'\ndescription='global'\ndeveloper_instructions='global'",
    );
    let local = workspace.path().join(".codex/agents/reviewer.toml");
    write(
        &local,
        "name='reviewer'\ndescription='workspace'\ndeveloper_instructions='local'",
    );

    let catalog =
        AgentCatalog::discover(&roots(home.path()), &[workspace.path().to_path_buf()]).unwrap();
    let global = catalog.resolve("reviewer", None, None).unwrap();
    assert_eq!(global.use_criteria, "global");
    let scoped = catalog
        .resolve("reviewer", Some(workspace.path()), None)
        .unwrap();
    assert_eq!(scoped.path, local);
    assert_eq!(scoped.use_criteria, "workspace");
}

#[test]
fn duplicate_harness_implementations_require_explicit_binding() {
    let home = TempDir::new().unwrap();
    write(
        &home.path().join(".codex/agents/reviewer.toml"),
        "name='reviewer'\ndescription='review'\ndeveloper_instructions='review'",
    );
    write(
        &home.path().join(".claude/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: review\n---\nReview",
    );
    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();

    let error = catalog.resolve("reviewer", None, None).unwrap_err();
    assert!(error.to_string().contains("multiple harnesses"));
    assert_eq!(
        catalog
            .resolve("reviewer", None, Some(Harness::ClaudeCode))
            .unwrap()
            .harness,
        Harness::ClaudeCode
    );
}

#[test]
fn rejects_malformed_native_profiles_without_guessing() {
    let home = TempDir::new().unwrap();
    write(
        &home.path().join(".codex/agents/broken.toml"),
        "name='broken'\ndescription='missing instructions'",
    );
    let error = AgentCatalog::discover(&roots(home.path()), &[]).unwrap_err();
    assert!(error.to_string().contains("developer_instructions"));
}

#[test]
fn rejects_duplicate_names_within_one_harness_scope() {
    let home = TempDir::new().unwrap();
    for file in ["first.toml", "second.toml"] {
        write(
            &home.path().join(".codex/agents").join(file),
            "name='reviewer'\ndescription='review'\ndeveloper_instructions='review'",
        );
    }
    let error = AgentCatalog::discover(&roots(home.path()), &[]).unwrap_err();
    assert!(error.to_string().contains("duplicate codex agent"));
}

#[test]
fn codex_activation_separates_catalog_metadata_from_root_config() {
    let home = TempDir::new().unwrap();
    write(
        &home.path().join(".codex/agents/reviewer.toml"),
        r#"name = "reviewer"
description = "Reviews correctness"
developer_instructions = "Review like an owner"
nickname_candidates = ["Atlas"]
model = "gpt-test"
model_reasoning_effort = "high"
"#,
    );
    let profile = AgentCatalog::discover(&roots(home.path()), &[])
        .unwrap()
        .resolve("reviewer", None, None)
        .unwrap();

    let NativeAgentActivation::CodexRoot(activation) = profile.activation().unwrap() else {
        panic!("expected Codex root activation");
    };
    assert_eq!(activation.developer_instructions, "Review like an owner");
    assert_eq!(activation.config["model"].as_str(), Some("gpt-test"));
    assert_eq!(
        activation.config["model_reasoning_effort"].as_str(),
        Some("high")
    );
    assert!(!activation.config.contains_key("name"));
    assert!(!activation.config.contains_key("description"));
    assert!(!activation.config.contains_key("nickname_candidates"));
}

#[test]
fn claude_and_opencode_use_their_native_agent_selector() {
    let home = TempDir::new().unwrap();
    write(
        &home.path().join(".claude/agents/designer.md"),
        "---\nname: designer\ndescription: Designs interfaces\n---\nPrompt",
    );
    write(
        &home.path().join(".config/opencode/agents/researcher.md"),
        "---\ndescription: Finds evidence\n---\nPrompt",
    );
    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    for slug in ["designer", "researcher"] {
        assert_eq!(
            catalog
                .resolve(slug, None, None)
                .unwrap()
                .activation()
                .unwrap(),
            NativeAgentActivation::NativeSelector {
                name: slug.to_string()
            }
        );
    }
}

#[test]
fn claude_discovers_nested_agents_and_requires_frontmatter_name() {
    let home = TempDir::new().unwrap();
    write(
        &home.path().join(".claude/agents/review/security-agent.md"),
        "---\nname: security\ndescription: Finds vulnerabilities\n---\nPrompt",
    );
    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    assert!(catalog.resolve("security", None, None).is_ok());

    write(
        &home.path().join(".claude/agents/nameless.md"),
        "---\ndescription: Missing identity\n---\nPrompt",
    );
    let error = AgentCatalog::discover(&roots(home.path()), &[]).unwrap_err();
    assert!(error.to_string().contains("requires \"name\""));
}

#[test]
fn opencode_uses_filename_and_excludes_non_root_agents() {
    let home = TempDir::new().unwrap();
    let dir = home.path().join(".config/opencode/agents");
    write(
        &dir.join("root-role.md"),
        "---\nname: ignored-name\ndescription: Root role\nmode: primary\n---\nPrompt",
    );
    write(
        &dir.join("worker.md"),
        "---\ndescription: Delegated only\nmode: subagent\n---\nPrompt",
    );
    write(
        &dir.join("retired.md"),
        "---\ndescription: Disabled\ndisable: true\n---\nPrompt",
    );

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    assert!(catalog.resolve("root-role", None, None).is_ok());
    assert!(catalog.resolve("ignored-name", None, None).is_err());
    assert!(catalog.resolve("worker", None, None).is_err());
    assert!(catalog.resolve("retired", None, None).is_err());
}
