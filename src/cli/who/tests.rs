use super::render::{render_who_once, render_who_plain, render_whoami};
use super::*;
use crate::session::{Harness, PeerStatusObservation, SessionObservation};
use crate::util::session_codename;

fn strip_ansi(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Register a local session into `session_state` and return its minted canonical
/// id. The daemon mints the id, so callers capture it from the returned snapshot
/// rather than asserting a fixed string.
fn register_local(
    store: &Store,
    slug: &str,
    pubkey: &str,
    project: &str,
    host: &str,
    rel_cwd: &str,
    harness_session_id: &str,
    observed_at: u64,
) -> String {
    let obs = SessionObservation {
        agent_slug: slug.to_string(),
        agent_pubkey: pubkey.to_string(),
        project: project.to_string(),
        host: host.to_string(),
        rel_cwd: rel_cwd.to_string(),
        harness: Harness::ClaudeCode,
        harness_session_id: Some(harness_session_id.to_string()),
        resume_id: None,
        tmux_pane: None,
        watch_pid: None,
        observed_at,
    };
    store
        .register_or_reassert_session(&obs)
        .unwrap()
        .session_id
        .as_str()
        .to_string()
}

/// Open a turn and seed a provisional title, so the local row carries a busy
/// title (the new model's equivalent of the deleted `set_agent_status`).
fn seed_busy_title(store: &Store, session_id: &str, title: &str, ts: u64) {
    let turn = store.start_turn(session_id, ts).unwrap().unwrap();
    store
        .seed_title_if_empty(session_id, turn.turn_id, title, ts)
        .unwrap()
        .unwrap();
}

/// Mirror a peer kind:30315 into `peer_session_state`.
#[allow(clippy::too_many_arguments)]
fn record_peer(
    store: &Store,
    pubkey: &str,
    slug: &str,
    project: &str,
    _native_session_id: &str,
    host: &str,
    rel_cwd: &str,
    title: &str,
    busy: bool,
    emitted_at: u64,
) {
    let obs = PeerStatusObservation {
        agent_pubkey: pubkey.to_string(),
        agent_slug: slug.to_string(),
        project: project.to_string(),
        host: host.to_string(),
        rel_cwd: rel_cwd.to_string(),
        title: title.to_string(),
        activity: String::new(),
        busy,
        emitted_at,
        observed_at: emitted_at,
    };
    store.record_peer_status(&obs).unwrap();
}

fn local_session(id: &str) -> crate::state::SessionRecord {
    crate::state::SessionRecord {
        session_id: id.to_string(),
        agent_slug: "coder".to_string(),
        agent_pubkey: "pk-coder".to_string(),
        project: "proj".to_string(),
        host: "laptop".to_string(),
        child_pid: Some(42),
        watch_pid: None,
        created_at: 1,
        alive: true,
        rel_cwd: String::new(),
        channel: String::new(),
    }
}

#[test]
fn who_snapshot_merges_local_and_peer_sessions() {
    let store = Store::open_memory().unwrap();
    // Local coder lives in session_state (the single source of truth).
    let coder_id = register_local(
        &store,
        "coder",
        "pk-coder",
        "proj",
        "laptop",
        "",
        "sid-coder",
        1_000,
    );
    // A peer echo of our own local session (same minted id) must be deduped.
    record_peer(
        &store, "pk-coder", "coder", "proj", &coder_id, "laptop", "", "", false, 1_000,
    );
    // A genuine remote peer on a different host.
    record_peer(
        &store,
        "pk-reviewer",
        "reviewer",
        "proj",
        "remote-session",
        "tower",
        "",
        "reviewing the patch",
        true,
        995,
    );

    // Daemon/viewer host is "laptop" → the local coder is same-machine; the
    // "tower" reviewer is a genuine remote.
    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();

    assert_eq!(snapshot.rows.len(), 2);
    let coder = snapshot
        .rows
        .iter()
        .find(|r| r.source == WhoSource::Local && r.slug == "coder")
        .expect("local coder row");
    let reviewer = snapshot
        .rows
        .iter()
        .find(|r| r.source == WhoSource::Peer && r.slug == "reviewer")
        .expect("peer reviewer row");
    // The self-echo (peer row mirroring our own canonical id) is hidden.
    assert!(!snapshot
        .rows
        .iter()
        .any(|r| r.source == WhoSource::Peer && r.session_id.as_str() == coder_id));

    // §8e same-host/remote: this machine's own session is NOT remote; a peer
    // on a different host IS.
    assert!(!coder.remote, "local session must never be remote");
    assert!(reviewer.remote, "tower peer must be remote vs laptop");

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.starts_with("proj\n\n"));
    // Host is shown for every agent now, including same-machine sessions. The
    // freshly-registered coder is idle (no turn opened yet).
    // Canonical agent reference `agent@host`; a local session shows no host paren.
    assert!(once.contains(&format!(
        "coder@laptop [session {}] - idle",
        session_codename(&coder_id)
    )));
    assert!(once.contains("coder@laptop"));
    // The genuine remote carries its host in the `@host` name plus a `(remote)` flag.
    assert!(once.contains("@tower"));
    assert!(once.contains("(remote)"));
}

#[test]
fn who_snapshot_uses_session_scoped_status_for_sibling_sessions() {
    let store = Store::open_memory().unwrap();
    // Two sibling sessions for the same agent, each with its own canonical id.
    let id_a = register_local(
        &store,
        "claude",
        "pk-claude",
        "proj",
        "laptop",
        "",
        "sid-a",
        1_000,
    );
    let id_b = register_local(
        &store,
        "claude",
        "pk-claude",
        "proj",
        "laptop",
        "",
        "sid-b",
        1_000,
    );
    // Each gets its own per-session title — proving status is session-scoped.
    seed_busy_title(&store, &id_a, "reading files", 1_000);
    seed_busy_title(&store, &id_b, "running tests", 1_000);

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let row_a = snapshot
        .rows
        .iter()
        .find(|r| r.session_id.as_str() == id_a)
        .expect("session-a row");
    let row_b = snapshot
        .rows
        .iter()
        .find(|r| r.session_id.as_str() == id_b)
        .expect("session-b row");
    assert_eq!(row_a.status, "reading files");
    assert_eq!(row_b.status, "running tests");
}

#[test]
fn who_snapshot_ignores_same_host_peer_echo_for_known_local_agent() {
    let store = Store::open_memory().unwrap();
    // A prior (now dead) local session for pk-claude is recorded in `sessions`,
    // so list_local_agent_pubkeys knows pk-claude is one of ours.
    let mut old = local_session("old-local");
    old.agent_slug = "claude".to_string();
    old.agent_pubkey = "pk-claude".to_string();
    old.alive = false;
    store.upsert_session(&old).unwrap();
    // The same identity arrives over the wire as a same-host peer echo.
    record_peer(
        &store,
        "pk-claude",
        "claude",
        "proj",
        "old-local",
        "laptop",
        "",
        "",
        false,
        1_000,
    );

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    assert!(
        snapshot.rows.is_empty(),
        "same-host peer echo for our own local identity should be hidden"
    );
}

#[test]
fn same_host_peer_is_not_remote() {
    // A sibling agent (e.g. codex@) on the SAME laptop arrives as a peer row;
    // it must NOT be tagged remote (the bug being fixed).
    let store = Store::open_memory().unwrap();
    record_peer(
        &store,
        "pk-codex",
        "codex",
        "proj",
        "sib",
        "laptop",
        "worktree1",
        "",
        false,
        1_000,
    );
    let snap = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let sib = snap
        .rows
        .iter()
        .find(|r| r.slug == "codex")
        .expect("sibling row");
    assert!(!sib.remote, "same-host peer must not be remote");
    assert_eq!(sib.rel_cwd, "worktree1");
    let once = strip_ansi(&render_who_once(&snap));
    assert!(
        once.contains("@laptop") && !once.contains("(remote)"),
        "same-host peer shows its host in the @host name with no remote flag"
    );
    assert!(once.contains("[worktree1]"), "rel_cwd shown in bracket");
}

#[test]
fn root_rel_cwd_has_no_bracket() {
    let store = Store::open_memory().unwrap();
    // rel_cwd "." (project root) → no [dir] bracket.
    record_peer(
        &store, "pk-a", "a", "proj", "r", "tower", ".", "", false, 1_000,
    );
    let snap = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let once = strip_ansi(&render_who_once(&snap));
    assert!(!once.contains("[.]"), "root cwd must not render a bracket");
    assert!(
        once.contains("@tower") && once.contains("(remote)"),
        "remote peer shows host in the @host name plus a remote flag"
    );
}

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
            pubkey: String::new(),
        }],
        other_projects: vec![],
        spawnable: vec![],
        channel_parent: None,
    };

    // --live uses render_who_once: same content, plus a dim quit-hint footer.
    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.contains("reviewer"));
    assert!(once.contains("reviewing the patch"));
}

#[test]
fn who_renderer_summarizes_other_projects() {
    let store = Store::open_memory().unwrap();
    record_peer(
        &store, "pk-a", "a", "proj", "s1", "laptop", "", "", false, 1_000,
    );
    record_peer(
        &store, "pk-b", "b", "other", "s2", "laptop", "", "", false, 1_000,
    );
    record_peer(
        &store, "pk-b", "b", "other", "s3", "laptop", "worktree", "", false, 1_001,
    );
    store
        .upsert_project_meta("other", "Other work", 1_000)
        .unwrap();

    let snap = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let once = strip_ansi(&render_who_once(&snap));

    // Peer rows no longer carry a native session id; their display id is the
    // pubkey, so the codename derives from "pk-a" (issue #5 §4 re-key).
    assert!(once.contains(&format!(
        "a@laptop [session {}] - idle",
        session_codename("pk-a")
    )));
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
            pubkey: String::new(),
        }],
        other_projects: vec![],
        spawnable: vec![],
        channel_parent: None,
    };

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.starts_with("all projects\n\n"));
    assert!(once.contains(&format!(
        "reviewer@tower [session {}] - idle",
        session_codename("remote-session")
    )));
}

#[test]
fn agent_renderer_uses_markdown_sections_and_session_table() {
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
    };

    let out = render_who_plain(&snapshot);
    assert!(out.starts_with("# tenex-edge who\n\nProject: proj\n\n## Sessions\n"));
    assert!(out.contains("| Agent | Session | Host | Title | Status |"));
    assert!(out.contains(&format!(
        "| reviewer@tower | `{}` | tower, remote [worktree] | Review plan | checking patch \\| tests |",
        session_codename("remote-session")
    )));
    assert!(out.contains("## Agents (for new sessions)"));
    // Agent table carries the byline ("When to use"), not the launch command.
    assert!(out.contains("| Agent | Host | When to use |"));
    assert!(out.contains("| codex | laptop | Use for autonomous coding tasks |"));
    assert!(!out.contains("| codex | laptop | `codex` |"));
    assert!(out.contains("## Other projects\n\n- other"));
}

#[test]
fn render_labels_session_room_as_channel_with_parent_project() {
    // When the scope is a per-session room, the header shows the room as the
    // current Channel and the work-root it's nested under as the Project.
    let snapshot = WhoSnapshot {
        project: "session-a1b2c3d4e5f60718".to_string(),
        now: 1000,
        rows: vec![],
        other_projects: vec![],
        spawnable: vec![],
        channel_parent: Some("lsjkd".to_string()),
    };
    let out = render_who_plain(&snapshot);
    assert!(
        out.contains("Channel: session-a1b2c3d4e5f60718 (your session room)"),
        "got: {out}"
    );
    assert!(out.contains("Project: lsjkd"), "got: {out}");
    // The room id must NOT be presented as the project.
    assert!(
        !out.contains("Project: session-a1b2c3d4e5f60718"),
        "got: {out}"
    );
}

#[test]
fn turn_start_fabric_block_uses_agent_markdown_renderer() {
    let store = Store::open_memory().unwrap();
    let _id = register_local(
        &store,
        "coder",
        "pk-coder",
        "proj",
        "laptop",
        "",
        "sid-coder",
        1_000,
    );
    let mutex = std::sync::Mutex::new(store);
    let mut blocks = Vec::new();

    push_turn_fabric_block(
        &mutex,
        &mut blocks,
        true,
        0,
        "proj",
        1_000,
        "laptop",
        "sid-coder",
    );

    let block = blocks.join("\n\n");
    assert!(block.contains("tenex-edge fabric — agents visible in this channel"));
    assert!(!block.contains("@<codename>"), "got: {block}");
    assert!(!block.contains("[session"), "got: {block}");
    assert!(block.contains("# tenex-edge who"));
    assert!(block.contains("| Agent | Host | Title | Status |"));
}

/// The shared delta renderer classifies appeared / changed (agent finished
/// busy→idle or a new title) / gone, project-scoped, with self-exclusion.
#[test]
fn build_status_delta_reports_appeared_changed_and_excludes_self() {
    let store = Store::open_memory().unwrap();
    // A peer that appears after the cursor.
    record_peer(
        &store,
        "pk-rev",
        "reviewer",
        "proj",
        "rev-1",
        "tower",
        "",
        "Review PR",
        false,
        1_000,
    );
    // The viewer's own local session — must be excluded from its own delta.
    let me_id = register_local(
        &store, "coder", "pk-coder", "proj", "laptop", "", "sid-me", 1_000,
    );
    seed_busy_title(&store, &me_id, "my work", 1_000);

    let lines = build_status_delta(&store, 500, "proj", 1_000, Some(&me_id));
    let joined = lines.join("\n");
    // The delta renders canonical presence lines: `* codename (agent@host) joined`.
    assert!(
        joined.contains("(reviewer@tower) joined"),
        "peer appearance must surface as a canonical `(reviewer@tower) joined` line: {joined}"
    );
    assert!(
        joined.trim_start().starts_with('*'),
        "delta lines are `* …` presence lines, not a table: {joined}"
    );
    assert!(
        !joined.contains(&session_codename(&me_id)),
        "viewer's own session must be excluded: {joined}"
    );
}

/// `whoami`'s agent-facing (non-TTY) render is a markdown identity card that
/// names the session's own codename and the `--to-session` form others use.
#[test]
fn render_whoami_card_names_self_and_addressing() {
    let card = serde_json::json!({
        "agent": "developer",
        "session_id": "sess-abc",
        "codename": session_codename("sess-abc"),
        "project": "tenex-edge",
        "host": "laptop",
        "rel_cwd": "worktree1",
        "pubkey": "deadbeef",
        "npub": "npub1xyz",
        "is_member": true,
        "working": true,
        "status": "Add whoami",
        "pending": 2,
        "created_at": 1_700_000_000u64,
    });
    let out = render_whoami(&card);
    let code = session_codename("sess-abc");
    assert!(
        out.contains(&format!("You are **developer** [session {code}]")),
        "card must name the agent + codename: {out}"
    );
    assert!(
        out.contains(&format!("`--to-session {code}`")),
        "card must show how others address this session: {out}"
    );
    assert!(out.contains("| Session ID | sess-abc |"), "raw id: {out}");
    assert!(
        out.contains("| Host | laptop [worktree1] |"),
        "host+cwd: {out}"
    );
    // The durable agent hex pubkey is shown (the wire address), never the npub.
    assert!(
        out.contains("| Pubkey | deadbeef |"),
        "hex durable pubkey shown, not npub: {out}"
    );
    assert!(
        !out.contains("npub1xyz"),
        "npub must NOT be rendered: {out}"
    );
    assert!(
        out.contains("| Status | Add whoami |"),
        "status title: {out}"
    );
    assert!(out.contains("| Chat | 2 pending |"), "pending count: {out}");
}
