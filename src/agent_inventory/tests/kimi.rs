use super::*;

#[test]
fn inventory_advertises_kimi_profiles_and_the_default_harness() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".kimi-code/agents/builder.md"),
        "---\nname: builder\ndescription: Implements and verifies changes\n---\nBuild",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();

    let inventory = AgentInventory::build(
        home.path(),
        &[Harness::Kimi],
        &HarnessesConfig::default(),
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
        ["builder", "kimi"]
    );
    let builder = inventory.find("builder").unwrap();
    assert_eq!(builder.harness, Harness::Kimi);
    assert_eq!(builder.use_criteria, "Implements and verifies changes");
    assert!(matches!(
        builder.source,
        AgentSource::DetectedProfile { .. }
    ));
}
