use super::render::{render_who_once, render_who_plain};
use super::*;
use crate::session::{Harness, PeerStatusObservation, SessionObservation};
use crate::util::session_short_code;

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
    native_session_id: &str,
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
        native_session_id: native_session_id.to_string(),
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
    let snapshot = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();

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
    assert!(once.contains(&format!(
        "coder [session {}] (laptop) - idle",
        session_short_code(&coder_id)
    )));
    assert!(once.contains("coder"));
    // The genuine remote is flagged `, remote` next to its hostname.
    assert!(once.contains("(tower, remote)"));
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

    let snapshot = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
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

    let snapshot = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
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
    let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
    let sib = snap
        .rows
        .iter()
        .find(|r| r.slug == "codex")
        .expect("sibling row");
    assert!(!sib.remote, "same-host peer must not be remote");
    assert_eq!(sib.rel_cwd, "worktree1");
    let once = strip_ansi(&render_who_once(&snap));
    assert!(
        once.contains("(laptop)") && !once.contains("(laptop, remote)"),
        "same-host peer shows its host with no remote flag"
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
    let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
    let once = strip_ansi(&render_who_once(&snap));
    assert!(!once.contains("[.]"), "root cwd must not render a bracket");
    assert!(
        once.contains("(tower, remote)"),
        "remote peer shows hostname with remote flag"
    );
}

#[test]
fn live_renderer_same_as_once_with_hint() {
    let snapshot = WhoSnapshot {
        project: "proj".to_string(),
        all: false,
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
            unread: 0,
        }],
        other_projects: vec![],
        spawnable: vec![],
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

    let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
    let once = strip_ansi(&render_who_once(&snap));

    assert!(once.contains(&format!(
        "a [session {}] (laptop) - idle",
        session_short_code("s1")
    )));
    assert!(once.contains("1 other agent(s) in other projects:"));
    assert!(once.contains("  * other - Other work"));
}

#[test]
fn who_all_projects_includes_project_in_agent_names() {
    let snapshot = WhoSnapshot {
        project: "*".to_string(),
        all: false,
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
            unread: 0,
        }],
        other_projects: vec![],
        spawnable: vec![],
    };

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.starts_with("all projects\n\n"));
    assert!(once.contains(&format!(
        "reviewer@other [session {}] (tower) - idle",
        session_short_code("remote-session")
    )));
}

#[test]
fn agent_renderer_uses_markdown_sections_and_session_table() {
    let snapshot = WhoSnapshot {
        project: "proj".to_string(),
        all: false,
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
            unread: 0,
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
        }],
    };

    let out = render_who_plain(&snapshot);
    assert!(out.starts_with("# tenex-edge who\n\nProject: proj\n\n## Sessions\n"));
    assert!(out.contains("| Agent | Session | Host | Title | Status |"));
    assert!(out.contains(&format!(
        "| reviewer | `{}` | tower, remote [worktree] | Review plan | checking patch \\| tests |",
        session_short_code("remote-session")
    )));
    assert!(out.contains("## Agents (for new sessions)"));
    assert!(out.contains("| codex | laptop | `codex` |"));
    assert!(out.contains("## Other projects\n\n- other"));
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
    assert!(block.contains("tenex-edge fabric — agents you can message"));
    assert!(block.contains("# tenex-edge who"));
    assert!(block.contains("| Agent | Session | Host | Title | Status |"));
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
    // The peer surfaces (it appeared after the cursor); the self session does not.
    assert!(
        joined.contains("reviewer") && joined.contains("joined"),
        "peer appearance must surface: {joined}"
    );
    assert!(
        joined.contains("Review PR · idle"),
        "appeared sessions must include their title/status: {joined}"
    );
    assert!(
        !joined.contains(&session_short_code(&me_id)),
        "viewer's own session must be excluded: {joined}"
    );
}
