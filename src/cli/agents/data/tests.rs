use super::*;
use crate::agent_catalog::{AgentCatalog, DiscoveryRoots};
use crate::agent_inventory::AgentInventory;
use std::path::Path;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn rows(home: &Path, installed: &[Harness], catalog: &AgentCatalog) -> Vec<AgentRow> {
    AgentInventory::build(home, installed, catalog, Some(home))
        .agents
        .into_iter()
        .map(project)
        .collect()
}

#[test]
fn combines_configured_native_and_generic_agents() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".claude/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews changes\n---\nReview",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    crate::identity::add_local_agent(home.path(), "writer", "codex", None, None, 1).unwrap();
    let rows = rows(
        home.path(),
        &[Harness::ClaudeCode, Harness::Codex],
        &catalog,
    );
    assert!(rows.iter().any(|row| row.slug == "reviewer"));
    assert!(rows.iter().any(|row| row.slug == "writer"));
    assert_eq!(
        rows.iter()
            .find(|row| row.slug == "codex")
            .unwrap()
            .description,
        "Generic Codex agent"
    );
}

#[test]
fn configured_native_profile_is_one_row_with_exact_profile_attached() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".claude/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews changes\n---\nReview",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    crate::identity::add_local_agent(home.path(), "reviewer", "claude-code", None, None, 1)
        .unwrap();
    let rows = rows(home.path(), &[Harness::ClaudeCode], &catalog);
    let reviewer = rows.iter().find(|row| row.slug == "reviewer").unwrap();
    assert_eq!(reviewer.kind, AgentKind::Configured);
    assert!(reviewer.native_profile.is_some());
}

#[test]
fn implicit_rows_have_no_preset() {
    let home = tempfile::tempdir().unwrap();
    write(
        &home.path().join(".config/opencode/agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews changes\n---\nReview",
    );
    let catalog = AgentCatalog::discover(&DiscoveryRoots::for_user_home(home.path()), &[]).unwrap();
    let rows = rows(home.path(), &[Harness::Opencode], &catalog);

    for row in rows {
        assert!(row.preset.is_none());
    }
}

#[test]
fn summary_is_single_line_and_bounded() {
    let row = AgentRow {
        slug: "reviewer".into(),
        agent_slug: "reviewer".into(),
        description: "First line\\n\\n<example>\nA very long native profile prompt follows".into(),
        harness: Harness::Codex,
        profile: None,
        preset: None,
        per_session_key: None,
        kind: AgentKind::NativeProfile,
        native_profile: None,
    };

    let summary = row.summary(32);

    assert!(summary.chars().count() <= 32);
    assert_eq!(summary, "First line <example> A very…");
}
