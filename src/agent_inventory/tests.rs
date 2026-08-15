use super::*;
use crate::agent_catalog::DiscoveryRoots;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn catalog_expands_profiles_and_includes_generic_agents() {
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
    let inventory = AgentInventory::build(
        home.path(),
        &[Harness::ClaudeCode, Harness::Codex],
        &catalog,
        None,
    );
    assert!(inventory.find("writer-claude").is_some());
    assert!(inventory.find("writer-codex").is_some());
    assert!(inventory.find("claude").is_some());
    assert!(inventory.find("codex").is_some());
}

#[test]
fn durable_agent_carries_canonical_harness_and_preset() {
    let home = tempfile::tempdir().unwrap();
    crate::identity::add_local_agent(
        home.path(),
        "writer",
        "codex",
        None,
        Some("unrestricted"),
        10,
    )
    .unwrap();
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    let inventory = AgentInventory::build(home.path(), &[Harness::Codex], &catalog, None);
    let writer = inventory.find("writer").unwrap();
    assert_eq!(writer.harness, Harness::Codex);
    assert!(matches!(
        &writer.source,
        AgentSource::Durable { preset: Some(name), .. } if name == "unrestricted"
    ));
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
        failures: vec![],
    };
    let decoded: AgentInventory =
        serde_json::from_value(serde_json::to_value(&inventory).unwrap()).unwrap();
    assert_eq!(decoded.agents, inventory.agents);
}
