use super::*;
use crate::agent_catalog::DiscoveryRoots;

#[path = "tests/hermes.rs"]
mod hermes;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn bundleless_catalog_expands_profiles_and_includes_generic_agents() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".codex/agents/writer.toml"),
        "name='writer'\ndescription='Writes with Codex'\ndeveloper_instructions='Write'",
    );
    write(
        &home.path().join(".claude/agents/writer.md"),
        "---\nname: writer\ndescription: Writes with Claude\n---\nWrite",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    let harnesses = HarnessesConfig::default();

    let inventory = AgentInventory::build(
        home.path(),
        &[
            Harness::ClaudeCode,
            Harness::Codex,
            Harness::Opencode,
            Harness::Goose,
            Harness::Hermes,
        ],
        &harnesses,
        &catalog,
        None,
    );

    assert!(inventory.failures.is_empty(), "{:?}", inventory.failures);
    assert_eq!(
        inventory
            .agents
            .iter()
            .map(|agent| agent.slug.as_str())
            .collect::<Vec<_>>(),
        [
            "claude",
            "codex",
            "goose",
            "hermes",
            "opencode",
            "writer-claude",
            "writer-codex"
        ]
    );
    assert_eq!(inventory.profile_choices("writer").len(), 2);
    assert_eq!(
        inventory.find("writer-claude").unwrap().use_criteria,
        "Writes with Claude"
    );
    assert_eq!(
        inventory.find("writer-codex").unwrap().use_criteria,
        "Writes with Codex"
    );
}

#[test]
fn configured_binding_collapses_profile_conflicts() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".codex/agents/writer.toml"),
        "name='writer'\ndescription='Writes'\ndeveloper_instructions='Write'",
    );
    write(
        &home.path().join(".claude/agents/writer.md"),
        "---\nname: writer\ndescription: Writes\n---\nWrite",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    let harnesses: HarnessesConfig = serde_json::from_str(
        r#"{
          "claude-pty":{"harness":"claude-code","transport":"pty"},
          "codex-pty":{"harness":"codex","transport":"pty"}
        }"#,
    )
    .unwrap();
    crate::identity::add_local_agent(home.path(), "writer", "codex-pty", None, 10).unwrap();

    let inventory = AgentInventory::build(
        home.path(),
        &[Harness::ClaudeCode, Harness::Codex],
        &harnesses,
        &catalog,
        None,
    );

    assert!(inventory.find("writer").is_some());
    assert!(inventory.find("writer-codex").is_none());
    assert!(inventory.find("writer-claude").is_none());
}

#[test]
fn sanitized_slug_reattaches_to_a_native_profile_with_spaces_in_its_name() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".claude/agents/ava-chen.md"),
        "---\nname: Ava Chen\ndescription: Investigative analyst\n---\nResearch",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    let harnesses: HarnessesConfig =
        serde_json::from_str(r#"{"claude-pty":{"harness":"claude-code","transport":"pty"}}"#)
            .unwrap();
    // Mirrors `cli::agents::editor`'s `persistable_slug`: the raw native
    // profile name ("Ava Chen") isn't a valid slug, so the configured entry
    // is persisted under its sanitized form instead.
    crate::identity::add_local_agent(home.path(), "ava-chen", "claude-pty", None, 10).unwrap();

    let inventory = AgentInventory::build(
        home.path(),
        &[Harness::ClaudeCode],
        &harnesses,
        &catalog,
        None,
    );

    assert_eq!(
        inventory
            .agents
            .iter()
            .map(|agent| agent.slug.as_str())
            .collect::<Vec<_>>(),
        ["ava-chen", "claude"],
        "the raw-named native profile must not also appear as its own unconfigured row"
    );
    let configured = inventory.find("ava-chen").unwrap();
    let AgentSource::Durable { native_profile, .. } = &configured.source else {
        panic!("expected a durable agent source");
    };
    assert_eq!(
        native_profile.as_ref().map(|profile| profile.slug.as_str()),
        Some("Ava Chen"),
        "the configured entry must reattach to its native profile despite the slug mismatch"
    );
}

#[test]
fn invalid_same_named_agent_does_not_shadow_available_harness() {
    let home = tempfile::tempdir().unwrap();
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    let harnesses: HarnessesConfig =
        serde_json::from_str(r#"{"codex-pty":{"harness":"codex","transport":"pty"}}"#).unwrap();
    crate::identity::add_local_agent(home.path(), "codex", "codex", None, 10).unwrap();

    let inventory =
        AgentInventory::build(home.path(), &[Harness::Codex], &harnesses, &catalog, None);

    let codex = inventory
        .find("codex")
        .expect("bare harness remains available");
    assert_eq!(codex.source, AgentSource::DetectedHarness);
    assert!(inventory
        .failures
        .iter()
        .any(|failure| failure.contains("codex")));
}

#[test]
fn daemon_inventory_wire_roundtrips_the_domain_model() {
    let inventory = AgentInventory {
        agents: vec![Agent {
            slug: "codex".into(),
            agent_slug: "codex".into(),
            harness: Harness::Codex,
            use_criteria: "General coding".into(),
            available_since: 7,
            source: AgentSource::DetectedHarness,
        }],
        failures: vec!["broken: unavailable".into()],
    };

    let value = serde_json::to_value(&inventory).unwrap();
    let decoded: AgentInventory = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.agents, inventory.agents);
    assert_eq!(decoded.failures, inventory.failures);
}
