use super::*;

#[test]
fn live_renderer_same_as_once_with_hint() {
    let snapshot = WhoSnapshot {
        project: "proj".to_string(),
        now: 1_000,
        rows: vec![WhoRow {
            source: WhoSource::Peer,
            fresh: true,
            slug: "reviewer".to_string(),
            project: "proj".to_string(),
            status: "reviewing the patch".to_string(),
            activity: String::new(),
            active: false,
            host: "tower".to_string(),
            session_id: "remote-session".to_string(),
            age_secs: Some(5),
            rel_cwd: String::new(),
            remote: false,
            attachable: false,
            work_root: "proj".to_string(),
            pubkey: String::new(),
        }],
        other_projects: vec![],
        spawnable: vec![],
        channel_parent: None,
        project_display: "proj".to_string(),
    };

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.contains("reviewer"));
    assert!(once.contains("reviewing the patch"));
}

#[test]
fn who_renderer_summarizes_other_projects() {
    let store = Store::open_memory().unwrap();
    // An idle agent in the current project.
    record_peer(&store, "pk-a", "a", "laptop", "", false, 1_000);
    // A root project "other" with its own about + one live agent.
    store
        .upsert_channel("other", "other", "Other work", "", 1_000)
        .unwrap();
    store
        .upsert_profile("pk-b", "b", "b", "laptop", false, 1)
        .unwrap();
    store
        .upsert_status(&Status {
            pubkey: "pk-b".to_string(),
            session_id: "sid-b".to_string(),
            channel_h: "other".to_string(),
            slug: "b".to_string(),
            title: String::new(),
            activity: String::new(),
            busy: false,
            last_seen: 1_000,
            updated_at: 1_000,
            expiration: 1_090,
        })
        .unwrap();

    let snap = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let once = strip_ansi(&render_who_once(&snap));

    assert!(once.contains("a (laptop) - idle"));
    assert!(!once.contains("[session"));
    assert!(once.contains("1 other agent(s) in other projects:"));
    assert!(once.contains("  * other - Other work"));
}

#[test]
fn who_all_projects_includes_project_in_agent_names() {
    let snapshot = WhoSnapshot {
        project: "*".to_string(),
        now: 1_000,
        rows: vec![WhoRow {
            source: WhoSource::Peer,
            fresh: true,
            slug: "reviewer".to_string(),
            project: "other".to_string(),
            status: String::new(),
            activity: String::new(),
            active: false,
            host: "tower".to_string(),
            session_id: "remote-session".to_string(),
            age_secs: Some(5),
            rel_cwd: String::new(),
            remote: false,
            attachable: false,
            work_root: "other".to_string(),
            pubkey: String::new(),
        }],
        other_projects: vec![],
        spawnable: vec![],
        channel_parent: None,
        project_display: "*".to_string(),
    };

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.starts_with("all projects\n\n"));
    assert!(once.contains("reviewer (tower) - idle"));
}

#[test]
fn agent_renderer_uses_markdown_sections_and_agent_table() {
    let snapshot = WhoSnapshot {
        project: "proj".to_string(),
        now: 1_000,
        rows: vec![WhoRow {
            source: WhoSource::Peer,
            fresh: true,
            slug: "reviewer".to_string(),
            project: "proj".to_string(),
            status: "Review plan".to_string(),
            activity: "checking patch | tests".to_string(),
            active: true,
            host: "tower".to_string(),
            session_id: "remote-session".to_string(),
            age_secs: Some(5),
            rel_cwd: "worktree".to_string(),
            remote: true,
            attachable: false,
            work_root: "proj".to_string(),
            pubkey: String::new(),
        }],
        other_projects: vec![OtherProjectSummary {
            project: "other".to_string(),
            agent_count: 1,
            agents: vec!["codex".to_string()],
            about: Some("ignored in agent renderer".to_string()),
        }],
        spawnable: vec![SpawnableRow {
            host: "laptop".to_string(),
            slug: "codex".to_string(),
            command: "codex".to_string(),
            byline: Some("Use for autonomous coding tasks".to_string()),
        }],
        channel_parent: None,
        project_display: "proj".to_string(),
    };

    let out = render_who_plain(&snapshot);
    assert!(out.starts_with("# tenex-edge who\n\nProject: proj\n\n## Agents in this channel\n"));
    assert!(out.contains("| Agent | Host | Title | Status |"));
    assert!(out.contains(
        "| reviewer | tower, remote [worktree] | Review plan | checking patch \\| tests |"
    ));
    assert!(!out.contains("[session"));
    assert!(!out.contains("remote-session"));
    assert!(out.contains("## Available agents"));
    assert!(out.contains("| Agent | Host | When to use |"));
    assert!(out.contains("| codex | laptop | Use for autonomous coding tasks |"));
    assert!(!out.contains("| codex | laptop | `codex` |"));
    assert!(out.contains("## Other projects\n\n- other"));
}

/// Concurrent same-agent instances now carry DISTINCT ordinal slugs
/// ("codex"/"codex1"), so the renderer prints the slug directly with no raw
/// session-id suffix (issue #99).
#[test]
fn agent_renderer_renders_ordinal_slugs_directly() {
    let snapshot = WhoSnapshot {
        project: "proj".to_string(),
        now: 1_000,
        rows: vec![
            WhoRow {
                source: WhoSource::Local,
                fresh: true,
                slug: "codex".to_string(),
                project: "proj".to_string(),
                status: "one".to_string(),
                activity: String::new(),
                active: false,
                host: "laptop".to_string(),
                session_id: "sess-a".to_string(),
                age_secs: Some(5),
                rel_cwd: String::new(),
                remote: false,
                attachable: false,
                work_root: "proj".to_string(),
                pubkey: String::new(),
            },
            WhoRow {
                source: WhoSource::Peer,
                fresh: true,
                slug: "codex1".to_string(),
                project: "proj".to_string(),
                status: "two".to_string(),
                activity: String::new(),
                active: false,
                host: "tower".to_string(),
                session_id: "sess-b".to_string(),
                age_secs: Some(5),
                rel_cwd: String::new(),
                remote: true,
                attachable: false,
                work_root: "proj".to_string(),
                pubkey: String::new(),
            },
        ],
        other_projects: vec![],
        spawnable: vec![],
        channel_parent: None,
        project_display: "proj".to_string(),
    };

    let out = render_who_plain(&snapshot);
    assert!(out.contains("| codex | laptop |"), "got: {out}");
    assert!(out.contains("| codex1 | tower, remote |"), "got: {out}");
    // No generated or raw session id ever surfaces as a name suffix.
    assert!(!out.contains("codex-"), "no generated suffix: {out}");
    assert!(!out.contains("sess-a"), "no raw session id: {out}");
    assert!(!out.contains("sess-b"), "no raw session id: {out}");
    assert!(!out.contains("| Agent | Session |"));
}

#[test]
fn render_labels_session_room_as_channel_with_parent_project() {
    let snapshot = WhoSnapshot {
        project: "session-a1b2c3d4e5f60718".to_string(),
        now: 1000,
        rows: vec![],
        other_projects: vec![],
        spawnable: vec![],
        channel_parent: Some("lsjkd".to_string()),
        project_display: "session-a1b2c3d4e5f60718".to_string(),
    };
    let out = render_who_plain(&snapshot);
    assert!(
        out.contains("Channel: session-a1b2c3d4e5f60718 (your session room)"),
        "got: {out}"
    );
    assert!(out.contains("Project: lsjkd"), "got: {out}");
    assert!(
        !out.contains("Project: session-a1b2c3d4e5f60718"),
        "got: {out}"
    );
}

#[test]
fn who_snapshot_exposes_work_root_for_session_room_rows() {
    let store = Store::open_memory().unwrap();
    // A session/task channel nested under project "proj" (parent set).
    store.upsert_channel("proj", "proj", "", "", 1_000).unwrap();
    store
        .upsert_channel("session-room", "session-room", "", "proj", 1_000)
        .unwrap();
    register_local_in(
        &store,
        "coder",
        "pk-coder",
        "session-room",
        "sid-coder",
        1_000,
    );

    let snapshot = load_who_snapshot(&store, Some("session-room"), 1_000, "laptop").unwrap();
    let row = snapshot.rows.first().expect("session-room row");
    assert_eq!(row.project, "session-room");
    assert_eq!(row.work_root, "proj");
}

#[test]
fn who_project_root_snapshot_includes_nested_channel_sessions() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("root", "root", "", "", 1_000).unwrap();
    store
        .upsert_channel("task", "Task", "", "root", 1_000)
        .unwrap();
    store
        .upsert_channel("leaf", "Leaf", "", "task", 1_000)
        .unwrap();
    register_local_in(&store, "coder", "pk-coder", "leaf", "sid-coder", 1_000);

    let snapshot = load_who_snapshot(&store, Some("root"), 1_000, "laptop").unwrap();

    let row = snapshot.rows.first().expect("nested channel row");
    assert_eq!(row.project, "leaf");
    assert_eq!(row.work_root, "root");
}
