use super::usage::{AgentUsage, AgentUsageMap};
use super::*;
use crate::test_env::EnvGuard;

fn write(path: &std::path::Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn conflict_selector_is_forwarded_for_daemon_side_realization() {
    let root = tempfile::tempdir().unwrap();
    let mosaico_home = root.path().join("mosaico");
    let codex_home = root.path().join(".codex");
    let mut env = EnvGuard::set("HOME", root.path());
    env.set_var("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("CODEX_HOME", &codex_home);
    env.set_var("XDG_CONFIG_HOME", root.path().join(".config"));
    write(
        &codex_home.join("agents/writer.toml"),
        "name='writer'\ndescription='Writes'\ndeveloper_instructions='Write'",
    );
    write(
        &root.path().join(".claude/agents/writer.md"),
        "---\nname: writer\ndescription: Writes\n---\nWrite",
    );

    let selection = resolve_fresh_agent("writer-codex", root.path()).unwrap();

    assert_eq!(selection.slug, "writer-codex");
    assert!(!mosaico_home.join("agents/writer.json").exists());
}

#[test]
fn menu_rows_are_aligned_single_line_and_bounded() {
    let inventory = crate::agent_inventory::AgentInventory {
        agents: vec![
            crate::agent_inventory::AvailableAgent {
                slug: "writing-partner".into(),
                agent_slug: "writing-partner".into(),
                harness: crate::session::Harness::ClaudeCode,
                use_criteria: "Drafts\\nrevises   and publishes ".repeat(20),
                available_since: 0,
                source: configured_source(),
            },
            crate::agent_inventory::AvailableAgent {
                slug: "codex".into(),
                agent_slug: "codex".into(),
                harness: crate::session::Harness::Codex,
                use_criteria: String::new(),
                available_since: 0,
                source: crate::agent_inventory::AgentSource::Generic,
            },
        ],
        failures: Vec::new(),
    };

    let usage = AgentUsageMap::new();
    let agents = ordered_agents(&inventory, &usage);
    let rows = menu_rows(&agents, &usage, 100);

    assert_eq!(rows[0].plain(), "codex  Generic Codex agent");
    assert_eq!(
        rows[0].description_harness,
        Some(crate::session::Harness::Codex)
    );
    assert!(!rows[1].plain().contains('\n'));
    assert!(rows[1]
        .description
        .starts_with("Drafts revises and publishes"));
    assert_eq!(rows[1].provenance, None);
}

#[test]
fn recent_count_then_last_use_determine_agent_order() {
    let agent = |slug: &str, agent_slug: &str| crate::agent_inventory::AvailableAgent {
        slug: slug.into(),
        agent_slug: agent_slug.into(),
        harness: crate::session::Harness::Codex,
        use_criteria: String::new(),
        available_since: 0,
        source: crate::agent_inventory::AgentSource::Generic,
    };
    let inventory = crate::agent_inventory::AgentInventory {
        agents: vec![
            agent("codex", "codex"),
            agent("writer-codex", "writer"),
            agent("grok", "grok"),
        ],
        failures: Vec::new(),
    };
    let usage = [("codex", 2, 90), ("writer", 3, 80), ("grok", 3, 95)]
        .into_iter()
        .map(|(agent_slug, recent_uses, last_used)| {
            (
                agent_slug.to_string(),
                AgentUsage {
                    agent_slug: agent_slug.to_string(),
                    recent_uses,
                    last_used,
                },
            )
        })
        .collect();

    let ordered = ordered_agents(&inventory, &usage)
        .into_iter()
        .map(|agent| agent.slug.as_str())
        .collect::<Vec<_>>();

    assert_eq!(ordered, ["grok", "writer-codex", "codex"]);
    let rows = menu_rows(&ordered_agents(&inventory, &usage), &usage, 100);
    assert_eq!(rows[0].description, "Generic Codex agent");
    assert_eq!(rows[0].usage.as_deref(), Some("3 uses / 30d · just now"));
    assert!(rows.iter().all(|row| row.provenance.is_none()));
}

#[test]
fn native_profile_description_precedes_colored_harness_provenance() {
    let agent = crate::agent_inventory::AvailableAgent {
        slug: "writer".into(),
        agent_slug: "writer".into(),
        harness: crate::session::Harness::ClaudeCode,
        use_criteria: "Drafts release notes".into(),
        available_since: 0,
        source: crate::agent_inventory::AgentSource::NativeProfile {
            profile: crate::agent_catalog::NativeAgentProfile {
                slug: "writer".into(),
                use_criteria: "Drafts release notes".into(),
                harness: crate::session::Harness::ClaudeCode,
                path: "/tmp/writer.md".into(),
                scope: crate::agent_catalog::AgentScope::Global,
                modified_at: 0,
            },
            persist_binding: false,
        },
    };

    let row = menu_row(&agent, &AgentUsageMap::new(), 100);

    assert_eq!(row.description, "Drafts release notes");
    assert_eq!(row.description_harness, None);
    assert_eq!(
        row.provenance,
        Some(AgentProvenance {
            label: "Claude profile".into(),
            harness: crate::session::Harness::ClaudeCode,
        })
    );
    assert_eq!(row.plain(), "writer  Drafts release notes · Claude profile");
}

#[test]
fn configured_agent_uses_byline_or_generic_configured_description_without_provenance() {
    let configured = |criteria: &str| crate::agent_inventory::AvailableAgent {
        slug: "writer".into(),
        agent_slug: "writer".into(),
        harness: crate::session::Harness::ClaudeCode,
        use_criteria: criteria.into(),
        available_since: 0,
        source: configured_source(),
    };

    let described = menu_row(
        &configured("Drafts release notes"),
        &AgentUsageMap::new(),
        100,
    );
    let generic = menu_row(&configured(""), &AgentUsageMap::new(), 100);

    assert_eq!(described.description, "Drafts release notes");
    assert_eq!(generic.description, "Configured agent");
    assert!(described.provenance.is_none());
    assert!(generic.provenance.is_none());
}

fn configured_source() -> crate::agent_inventory::AgentSource {
    crate::agent_inventory::AgentSource::Configured {
        bundle: "claude-pty".into(),
        profile: None,
        per_session_key: true,
        native_profile: None,
    }
}
