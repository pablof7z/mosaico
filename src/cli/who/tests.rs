use super::render::render_who_once;
use super::*;
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
    store
        .upsert_session(&local_session("local-session"))
        .unwrap();
    store.touch_session("local-session", 1_000).unwrap();
    store
        .upsert_peer_session(
            "local-session",
            "pk-coder",
            "coder",
            "proj",
            "laptop",
            "",
            1_000,
        )
        .unwrap();
    store
        .upsert_peer_session(
            "remote-session",
            "pk-reviewer",
            "reviewer",
            "proj",
            "tower",
            "",
            995,
        )
        .unwrap();
    store
        .set_agent_status(
            "pk-reviewer",
            "proj",
            Some("remote-session"),
            "reviewing the patch",
            995,
        )
        .unwrap();

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
    assert!(!snapshot
        .rows
        .iter()
        .any(|r| r.source == WhoSource::Peer && r.session_id.as_str() == "local-session"));

    // §8e same-host/remote: this machine's own session is NOT remote; a peer
    // on a different host IS.
    assert!(!coder.remote, "local session must never be remote");
    assert!(reviewer.remote, "tower peer must be remote vs laptop");

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.starts_with("proj\n\n"));
    assert!(once.contains(&format!(
        "coder [session {}] - idle",
        session_short_code("local-session")
    )));
    assert!(once.contains("coder"));
    // The remote tag shows the actual hostname only for the genuine remote.
    assert!(once.contains("(tower)"));
}

#[test]
fn who_snapshot_uses_session_scoped_status_for_sibling_sessions() {
    let store = Store::open_memory().unwrap();
    let mut a = local_session("session-a");
    a.agent_slug = "claude".to_string();
    a.agent_pubkey = "pk-claude".to_string();
    a.created_at = 1;
    let mut b = a.clone();
    b.session_id = "session-b".to_string();
    b.created_at = 2;
    store.upsert_session(&a).unwrap();
    store.upsert_session(&b).unwrap();
    store.touch_session("session-a", 1_000).unwrap();
    store.touch_session("session-b", 1_000).unwrap();
    store
        .set_agent_status("pk-claude", "proj", Some("session-a"), "reading files", 995)
        .unwrap();
    store
        .set_agent_status("pk-claude", "proj", Some("session-b"), "running tests", 996)
        .unwrap();

    let snapshot = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
    let row_a = snapshot
        .rows
        .iter()
        .find(|r| r.session_id.as_str() == "session-a")
        .expect("session-a row");
    let row_b = snapshot
        .rows
        .iter()
        .find(|r| r.session_id.as_str() == "session-b")
        .expect("session-b row");
    assert_eq!(row_a.status, "reading files");
    assert_eq!(row_b.status, "running tests");
}

#[test]
fn who_snapshot_ignores_same_host_peer_echo_for_known_local_agent() {
    let store = Store::open_memory().unwrap();
    let mut old = local_session("old-local");
    old.agent_slug = "claude".to_string();
    old.agent_pubkey = "pk-claude".to_string();
    old.alive = false;
    store.upsert_session(&old).unwrap();
    store
        .upsert_peer_session(
            "old-local",
            "pk-claude",
            "claude",
            "proj",
            "laptop",
            "",
            1_000,
        )
        .unwrap();

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
    store
        .upsert_peer_session(
            "sib",
            "pk-codex",
            "codex",
            "proj",
            "laptop",
            "worktree1",
            1_000,
        )
        .unwrap();
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
        !once.contains("(laptop)"),
        "no remote tag for same-host peer"
    );
    assert!(once.contains("[worktree1]"), "rel_cwd shown in bracket");
}

#[test]
fn root_rel_cwd_has_no_bracket() {
    let store = Store::open_memory().unwrap();
    // rel_cwd "." (project root) → no [dir] bracket.
    store
        .upsert_peer_session("r", "pk-a", "a", "proj", "tower", ".", 1_000)
        .unwrap();
    let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
    let once = strip_ansi(&render_who_once(&snap));
    assert!(!once.contains("[.]"), "root cwd must not render a bracket");
    assert!(once.contains("(tower)"), "remote peer shows hostname");
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
            host: "tower".to_string(),
            session_id: SessionId::from("remote-session"),
            age_secs: Some(5),
            rel_cwd: String::new(),
            remote: false,
        }],
        other_projects: vec![],
    };

    // --live uses render_who_once: same content, plus a dim quit-hint footer.
    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.contains("reviewer"));
    assert!(once.contains("reviewing the patch"));
}

#[test]
fn who_renderer_summarizes_other_projects() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_peer_session("s1", "pk-a", "a", "proj", "laptop", "", 1_000)
        .unwrap();
    store
        .upsert_peer_session("s2", "pk-b", "b", "other", "laptop", "", 1_000)
        .unwrap();
    store
        .upsert_peer_session("s3", "pk-b", "b", "other", "laptop", "worktree", 1_001)
        .unwrap();
    store
        .upsert_project_meta("other", "Other work", 1_000)
        .unwrap();

    let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
    let once = strip_ansi(&render_who_once(&snap));

    assert!(once.contains(&format!("a [session {}] - idle", session_short_code("s1"))));
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
            host: "tower".to_string(),
            session_id: SessionId::from("remote-session"),
            age_secs: Some(5),
            rel_cwd: String::new(),
            remote: false,
        }],
        other_projects: vec![],
    };

    let once = strip_ansi(&render_who_once(&snapshot));
    assert!(once.starts_with("all projects\n\n"));
    assert!(once.contains(&format!(
        "reviewer@other [session {}] - idle",
        session_short_code("remote-session")
    )));
}
