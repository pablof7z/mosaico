use super::*;

#[test]
fn inventory_advertises_hermes_profiles_and_the_default_harness() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".hermes/profiles/builder/profile.yaml"),
        "description: Implements scoped changes and validates the result.\n",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();

    let inventory = AgentInventory::build(
        home.path(),
        &[Harness::Hermes],
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
        ["builder", "hermes"]
    );
    let builder = inventory.find("builder").unwrap();
    assert_eq!(builder.harness, Harness::Hermes);
    assert_eq!(
        builder.use_criteria,
        "Implements scoped changes and validates the result."
    );
    assert!(matches!(
        builder.source,
        AgentSource::DetectedProfile { .. }
    ));
}
