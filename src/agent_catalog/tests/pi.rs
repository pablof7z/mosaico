use super::*;

#[test]
fn discovers_settings_json_profiles_with_pi_suffixed_slugs() {
    let home = TempDir::new().unwrap();
    let pi = home.path().join(".pi");
    write(&pi.join("architect/settings.json"), "{}");
    write(&pi.join("agent/settings.json"), "{}");

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();

    let architect = catalog
        .resolve("architect-pi", None, Some(Harness::Pi))
        .unwrap();
    assert_eq!(architect.harness, Harness::Pi);
    assert_eq!(architect.path, pi.join("architect"));
    let default = catalog
        .resolve("agent-pi", None, Some(Harness::Pi))
        .unwrap();
    assert_eq!(default.path, pi.join("agent"));
    assert_eq!(catalog.slugs(), ["agent-pi", "architect-pi"]);
}

#[test]
fn ignores_pi_data_directories_and_invalid_profile_names() {
    let home = TempDir::new().unwrap();
    let pi = home.path().join(".pi");
    for ignored in ["cache", "loops", "stt-recordings", "tasks"] {
        std::fs::create_dir_all(pi.join(ignored)).unwrap();
    }
    write(&pi.join("UPPER/settings.json"), "{}");
    std::fs::create_dir_all(pi.join("no-marker")).unwrap();
    write(&pi.join("regular-file"), "not a profile");

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();

    assert_eq!(catalog.slugs(), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn follows_symlinked_pi_profiles() {
    let home = TempDir::new().unwrap();
    let external = home.path().join("elsewhere/janitor");
    write(&external.join("settings.json"), "{}");
    let pi = home.path().join(".pi");
    std::fs::create_dir_all(&pi).unwrap();
    std::os::unix::fs::symlink(&external, pi.join("repo-janitor")).unwrap();

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    let discovered = catalog
        .resolve("repo-janitor-pi", None, Some(Harness::Pi))
        .unwrap();
    assert_eq!(discovered.path, pi.join("repo-janitor"));
}

#[test]
fn inventory_projects_pi_profiles_as_launchable_pi_agents() {
    let home = TempDir::new().unwrap();
    write(&home.path().join(".pi/architect/settings.json"), "{}");
    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    let mosaico_home = home.path().join(".mosaico");
    std::fs::create_dir_all(&mosaico_home).unwrap();

    let inventory = crate::agent_inventory::AgentInventory::build(
        &mosaico_home,
        &[Harness::Pi],
        &catalog,
        None,
    );

    let agent = inventory.find("architect-pi").expect("pi profile agent");
    assert_eq!(agent.agent_slug, "architect-pi");
    assert_eq!(agent.harness, Harness::Pi);
    assert!(matches!(
        agent.source,
        crate::agent_inventory::AgentSource::DetectedProfile { .. }
    ));
}
