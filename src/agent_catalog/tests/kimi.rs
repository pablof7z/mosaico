use super::*;

#[test]
fn discovers_recursive_kimi_profiles_with_brand_root_precedence() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".agents/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Shared reviewer\n---\nShared prompt",
    );
    let brand = home.path().join(".kimi-code/agents/team/strict.md");
    write(
        &brand,
        "---\nname: reviewer\ndescription: Kimi reviewer\n---\nKimi prompt",
    );
    write(
        &home.path().join(".kimi-code/agents/planner.md"),
        "---\ndescription: Plans changes\n---\nPlan",
    );

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    let reviewer = catalog
        .resolve("reviewer", None, Some(Harness::Kimi))
        .unwrap();
    assert_eq!(reviewer.path, brand);
    assert_eq!(reviewer.use_criteria, "Kimi reviewer");
    assert_eq!(
        reviewer.activation().unwrap(),
        NativeAgentActivation::NativeSelector {
            name: "reviewer".into()
        }
    );
    assert_eq!(
        catalog
            .resolve("planner", None, Some(Harness::Kimi))
            .unwrap()
            .path
            .file_stem()
            .and_then(|value| value.to_str()),
        Some("planner")
    );
}

#[test]
fn workspace_kimi_profile_overrides_the_user_profile() {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".kimi-code/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: User reviewer\n---\nUser",
    );
    let local = workspace.path().join(".kimi-code/agents/reviewer.md");
    write(
        &local,
        "---\nname: reviewer\ndescription: Project reviewer\n---\nProject",
    );

    let catalog =
        AgentCatalog::discover(&roots(home.path()), &[workspace.path().to_path_buf()]).unwrap();
    assert_eq!(
        catalog
            .resolve("reviewer", Some(workspace.path()), Some(Harness::Kimi))
            .unwrap()
            .path,
        local
    );
}
