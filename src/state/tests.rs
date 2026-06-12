use super::*;

fn sample_session(id: &str) -> SessionRecord {
    SessionRecord {
        session_id: id.into(),
        agent_slug: "coder".into(),
        agent_pubkey: "pk-coder".into(),
        project: "proj".into(),
        host: "laptop".into(),
        child_pid: Some(42),
        watch_pid: Some(7),
        created_at: 1000,
        alive: true,
        rel_cwd: String::new(),
    }
}

#[test]
fn session_roundtrip_and_death() {
    let s = Store::open_memory().unwrap();
    s.upsert_session(&sample_session("sess-1")).unwrap();
    assert_eq!(
        s.get_session("sess-1").unwrap().unwrap(),
        sample_session("sess-1")
    );
    assert_eq!(s.list_alive_sessions().unwrap().len(), 1);
    s.mark_session_dead("sess-1").unwrap();
    assert!(s.list_alive_sessions().unwrap().is_empty());
    assert!(!s.get_session("sess-1").unwrap().unwrap().alive);
}

#[test]
fn inbox_is_idempotent_per_session() {
    let s = Store::open_memory().unwrap();
    let row = InboxRow {
        mention_event_id: "evt-1".into(),
        target_session: "sess-A".into(),
        from_pubkey: "pk".into(),
        from_slug: "reviewer".into(),
        project: "proj".into(),
        body: "look here".into(),
        created_at: 5,
        from_session: "sender-A".into(),
        subject: String::new(),
        branch: String::new(),
        commit: String::new(),
        dirty: 0,
        host: String::new(),
    };
    assert!(s.enqueue_mention(&row).unwrap()); // new
    assert!(!s.enqueue_mention(&row).unwrap()); // duplicate ignored
                                                // same mention, different session = distinct delivery
    let mut other = row.clone();
    other.target_session = "sess-B".into();
    assert!(s.enqueue_mention(&other).unwrap());

    let drained = s.drain_inbox("sess-A").unwrap();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].body, "look here");
    assert!(s.drain_inbox("sess-A").unwrap().is_empty()); // delivered once
    assert_eq!(s.drain_inbox("sess-B").unwrap().len(), 1);
}

/// Bug C (agent-scoped sender resolution): the latest-alive fallback must be
/// scoped to the invoking agent, not the most-recently-active session of ANY
/// agent in the project. Otherwise a `claude` send is recorded as `opencode`
/// merely because opencode was the latest-active session.
#[test]
fn latest_alive_session_is_agent_scoped() {
    let s = Store::open_memory().unwrap();
    let mut claude = sample_session("sess-claude");
    claude.agent_slug = "claude".into();
    claude.agent_pubkey = "pk-claude".into();
    claude.created_at = 100;
    s.upsert_session(&claude).unwrap();

    let mut opencode = sample_session("sess-opencode");
    opencode.agent_slug = "opencode".into();
    opencode.agent_pubkey = "pk-opencode".into();
    opencode.created_at = 200; // more recently active
    s.upsert_session(&opencode).unwrap();

    // Agent-agnostic lookup returns opencode (the latest active) — the BUG.
    assert_eq!(
        s.latest_alive_session_for_project("proj")
            .unwrap()
            .unwrap()
            .agent_slug,
        "opencode"
    );
    // Agent-scoped lookup honors the invoking agent.
    assert_eq!(
        s.latest_alive_session_for_agent_in_project("claude", "proj")
            .unwrap()
            .unwrap()
            .agent_slug,
        "claude"
    );
    assert_eq!(
        s.latest_alive_session_for_agent_in_project("opencode", "proj")
            .unwrap()
            .unwrap()
            .agent_slug,
        "opencode"
    );
    // No alive session for an unknown agent.
    assert!(s
        .latest_alive_session_for_agent_in_project("codex", "proj")
        .unwrap()
        .is_none());
}

#[test]
fn resolve_with_project_scope_prefers_matching_presence() {
    let s = Store::open_memory().unwrap();
    s.upsert_peer_session(
        "sess-x",
        "pk-from-presence",
        "reviewer",
        "proj",
        "host",
        "",
        1,
    )
    .unwrap();
    assert_eq!(
        s.resolve_agent_pubkey("reviewer", Some("proj"))
            .unwrap()
            .as_deref(),
        Some("pk-from-presence")
    );
    s.upsert_profile("pk-from-profile", "reviewer", "host", 2)
        .unwrap();
    assert_eq!(
        s.resolve_agent_pubkey("reviewer", Some("proj"))
            .unwrap()
            .as_deref(),
        Some("pk-from-presence")
    );
    assert_eq!(
        s.resolve_agent_pubkey("reviewer", Some("other"))
            .unwrap()
            .as_deref(),
        None
    );
    assert_eq!(
        s.resolve_agent_pubkey("reviewer", None).unwrap().as_deref(),
        Some("pk-from-profile")
    );
}

#[test]
fn peer_freshness_and_prune() {
    let s = Store::open_memory().unwrap();
    s.upsert_peer_session("old", "pk1", "stale", "proj", "h", "", 100)
        .unwrap();
    s.upsert_peer_session("new", "pk2", "live", "proj", "h", "", 1000)
        .unwrap();
    // since=500 → only the fresh one is "live"
    let live = s.list_peer_sessions(Some("proj"), 500).unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].slug, "live");
    // since=0 → both
    assert_eq!(s.list_peer_sessions(Some("proj"), 0).unwrap().len(), 2);
    // prune removes the stale one
    assert_eq!(s.prune_peer_sessions(500).unwrap(), 1);
    assert_eq!(s.list_peer_sessions(Some("proj"), 0).unwrap().len(), 1);
}

#[test]
fn rel_cwd_persists_on_peer_and_own_sessions() {
    let s = Store::open_memory().unwrap();
    // Peer session learns rel_cwd from presence.
    s.upsert_peer_session("p1", "pk", "rev", "proj", "tower", "worktree2", 1_000)
        .unwrap();
    let peers = s.list_peer_sessions(Some("proj"), 0).unwrap();
    assert_eq!(peers[0].rel_cwd, "worktree2");
    // Updating keeps the latest rel_cwd.
    s.upsert_peer_session("p1", "pk", "rev", "proj", "tower", "sub/dir", 1_001)
        .unwrap();
    assert_eq!(
        s.list_peer_sessions(Some("proj"), 0).unwrap()[0].rel_cwd,
        "sub/dir"
    );

    // Own session stores + reads back rel_cwd (needed by reconcile).
    s.upsert_session(&sample_session("mine")).unwrap();
    let mut rec = sample_session("mine");
    rec.rel_cwd = "worktree1".into();
    s.upsert_session(&rec).unwrap();
    assert_eq!(s.get_session("mine").unwrap().unwrap().rel_cwd, "worktree1");
}

#[test]
fn rel_cwd_migration_is_idempotent_on_reopen() {
    // Opening an on-disk db twice must not fail on the guarded ALTER TABLE
    // (the column already exists the second time).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let s = Store::open(&path).unwrap();
        let mut rec = sample_session("m");
        rec.rel_cwd = "wt".into();
        s.upsert_session(&rec).unwrap();
    }
    let s2 = Store::open(&path).unwrap();
    assert_eq!(s2.get_session("m").unwrap().unwrap().rel_cwd, "wt");
}

mod directories;
