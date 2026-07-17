use crate::identity::{
    add_local_agent, list_invitable_agents, list_local_agents, set_local_agent_byline,
};

#[test]
fn byline_reads_only_the_canonical_field() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("agents")).unwrap();
    std::fs::write(
        dir.path().join("agents/a.json"),
        r#"{"slug":"a","created_at":1,"perSessionKey":true,"harness":"claude","byline":"front-line triage"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("agents/b.json"),
        r#"{"slug":"b","created_at":1,"perSessionKey":true,"harness":"claude","useCriteria":"use for deep research"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("agents/c.json"),
        r#"{"slug":"c","created_at":1,"perSessionKey":true,"harness":"claude","agent":{"description":"writes social posts"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("agents/d.json"),
        r#"{"slug":"d","created_at":1,"perSessionKey":true,"harness":"claude"}"#,
    )
    .unwrap();

    let agents = list_local_agents(dir.path());
    let byline = |slug: &str| {
        agents
            .iter()
            .find(|a| a.0 == slug)
            .and_then(|a| a.3.clone())
    };
    assert_eq!(byline("a").as_deref(), Some("front-line triage"));
    assert_eq!(byline("b"), None);
    assert_eq!(byline("c"), None);
    assert_eq!(byline("d"), None);
}

#[test]
fn set_local_agent_byline_updates_invitable_roster() {
    let dir = tempfile::tempdir().unwrap();
    add_local_agent(dir.path(), "reviewer", "claude", None, 1).unwrap();

    set_local_agent_byline(
        dir.path(),
        "reviewer",
        Some("use for skeptical code review".into()),
    )
    .unwrap();

    let agents = list_local_agents(dir.path());
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].0, "reviewer");
    assert_eq!(
        agents[0].3.as_deref(),
        Some("use for skeptical code review")
    );

    let roster = list_invitable_agents(dir.path());
    assert_eq!(
        roster[0].1.as_deref(),
        Some("use for skeptical code review")
    );
}
