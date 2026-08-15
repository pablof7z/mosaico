use crate::identity::{
    add_local_agent, list_invitable_agents, list_local_agents, set_local_agent_byline,
};

#[test]
fn byline_reads_only_the_canonical_field() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("agents")).unwrap();
    std::fs::write(
        dir.path().join("agents/a.json"),
        r#"{"slug":"a","created_at":1,"perSessionKey":true,"harness":"claude-code","byline":"front-line triage"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("agents/b.json"),
        r#"{"slug":"b","created_at":1,"perSessionKey":true,"harness":"claude-code","useCriteria":"use for deep research"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("agents/c.json"),
        r#"{"slug":"c","created_at":1,"perSessionKey":true,"harness":"claude-code","agent":{"description":"writes social posts"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("agents/d.json"),
        r#"{"slug":"d","created_at":1,"perSessionKey":true,"harness":"claude-code"}"#,
    )
    .unwrap();

    let agents = list_local_agents(dir.path());
    let byline = |slug: &str| {
        agents
            .iter()
            .find(|agent| agent.slug == slug)
            .and_then(|agent| agent.byline.clone())
    };
    assert_eq!(byline("a").as_deref(), Some("front-line triage"));
    assert_eq!(byline("b"), None);
    assert_eq!(byline("c"), None);
    assert_eq!(byline("d"), None);
}

#[test]
fn set_local_agent_byline_updates_invitable_roster() {
    let dir = tempfile::tempdir().unwrap();
    add_local_agent(dir.path(), "reviewer", "claude-code", None, None, 1).unwrap();

    set_local_agent_byline(
        dir.path(),
        "reviewer",
        Some("use for skeptical code review".into()),
    )
    .unwrap();

    let agents = list_local_agents(dir.path());
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].slug, "reviewer");
    assert_eq!(
        agents[0].byline.as_deref(),
        Some("use for skeptical code review")
    );

    let roster = list_invitable_agents(dir.path());
    assert_eq!(
        roster[0].1.as_deref(),
        Some("use for skeptical code review")
    );
}
