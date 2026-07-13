use super::*;
use crate::state::{Identity, RegisterSession, Status};
use crate::who_snapshot::{load_who_snapshot, WhoSource};

/// Register a local session (daemon mints the canonical id).
fn register_local(store: &Store, slug: &str, pubkey: &str, ext_id: &str, ts: u64) -> String {
    register_local_in(store, slug, pubkey, "proj", ext_id, ts)
}

fn register_local_in(
    store: &Store,
    slug: &str,
    pubkey: &str,
    channel: &str,
    ext_id: &str,
    ts: u64,
) -> String {
    store
        .register_session(&RegisterSession {
            harness: "claude-code".to_string(),
            external_id_kind: "harness_session".to_string(),
            external_id: ext_id.to_string(),
            agent_pubkey: pubkey.to_string(),
            agent_slug: slug.to_string(),
            channel_h: channel.to_string(),
            child_pid: Some(42),
            transcript_path: None,
            resume_id: String::new(),
            now: ts,
        })
        .unwrap()
}

/// Set a session's local pre-publish draft title. Local rows fall back to this
/// when no kind:30315 has been published yet.
fn seed_draft_title(store: &Store, session_id: &str, title: &str, ts: u64) {
    store
        .set_session_distill(session_id, title, "", ts)
        .unwrap();
}

/// Record a peer (or our own published) status as a kind:30315 in `relay_status`,
/// plus a kind:0 carrying its host so remoteness can be derived.
#[allow(clippy::too_many_arguments)]
fn record_peer(
    store: &Store,
    pubkey: &str,
    slug: &str,
    host: &str,
    title: &str,
    busy: bool,
    ts: u64,
) {
    store
        .upsert_profile(pubkey, slug, slug, host, false, 1)
        .unwrap();
    store
        .upsert_status(&Status {
            pubkey: pubkey.to_string(),
            session_id: format!("sid-{slug}"),
            channel_h: "proj".to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            activity: String::new(),
            busy,
            last_seen: ts,
            updated_at: ts,
            expiration: ts + 90,
        })
        .unwrap();
}

/// Declare `pubkey` as a key this daemon signs as, so a relay echo of our own
/// status is dropped from the peer set.
fn own_identity(store: &Store, pubkey: &str, slug: &str) {
    store
        .upsert_identity(&Identity {
            pubkey: pubkey.to_string(),
            agent_slug: slug.to_string(),
            codename: "willow-echo-042".to_string(),
            session_id: String::new(),
            channel_h: "proj".to_string(),
            native_id: String::new(),
            alive: true,
            created_at: 1,
        })
        .unwrap();
}

#[test]
fn who_snapshot_merges_local_and_peer_sessions() {
    let store = Store::open_memory().unwrap();
    // Local coder is a hosted session; pk-coder is one of our signing keys.
    own_identity(&store, "pk-coder", "coder");
    let _coder_sid = register_local(&store, "coder", "pk-coder", "sid-coder", 1_000);
    // With no bound identity row, the local row displays the public session handle.
    let coder_handle = "coder".to_string();
    // A relay echo of our own status (pk-coder) must be deduped out of peers.
    record_peer(&store, "pk-coder", "coder", "laptop", "", false, 1_000);
    // A genuine remote peer on a different host.
    record_peer(
        &store,
        "pk-reviewer",
        "reviewer",
        "tower",
        "reviewing the patch",
        true,
        1_000,
    );

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();

    assert_eq!(snapshot.rows.len(), 2);
    let coder = snapshot
        .rows
        .iter()
        .find(|r| r.source == WhoSource::Local && r.slug == coder_handle)
        .expect("local coder row");
    let reviewer = snapshot
        .rows
        .iter()
        .find(|r| r.source == WhoSource::Peer && r.slug == "reviewer")
        .expect("peer reviewer row");
    // Our own relay echo is not also a peer row.
    assert!(!snapshot
        .rows
        .iter()
        .any(|r| r.source == WhoSource::Peer && r.pubkey == "pk-coder"));

    assert!(!coder.remote, "local session must never be remote");
    assert!(reviewer.remote, "tower peer must be remote vs laptop");
}

#[test]
fn who_snapshot_uses_session_draft_title_for_sibling_sessions() {
    let store = Store::open_memory().unwrap();
    // Two sibling sessions for the same agent, each with its own canonical id and
    // its own local draft title (no kind:30315 published yet).
    let id_a = register_local(&store, "claude", "pk-claude", "sid-a", 1_000);
    let id_b = register_local(&store, "claude", "pk-claude", "sid-b", 1_000);
    seed_draft_title(&store, &id_a, "reading files", 1_000);
    seed_draft_title(&store, &id_b, "running tests", 1_000);

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let row_a = snapshot
        .rows
        .iter()
        .find(|r| r.session_id == id_a)
        .expect("session-a row");
    let row_b = snapshot
        .rows
        .iter()
        .find(|r| r.session_id == id_b)
        .expect("session-b row");
    assert_eq!(row_a.status, "reading files");
    assert_eq!(row_b.status, "running tests");
}

#[test]
fn who_snapshot_ignores_relay_echo_for_known_local_agent() {
    let store = Store::open_memory().unwrap();
    // pk-claude is one of our signing keys, but no live local session exists.
    own_identity(&store, "pk-claude", "claude");
    // The same identity arrives over the wire as a relay echo.
    record_peer(&store, "pk-claude", "claude", "laptop", "", false, 1_000);

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    assert!(
        snapshot.rows.is_empty(),
        "relay echo for our own identity should be hidden"
    );
}

#[test]
fn who_snapshot_hides_archived_channel_presence() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("proj", "proj", "", "", 1).unwrap();
    store
        .upsert_channel("archived", "archived", "[ARCHIVED] done", "proj", 1)
        .unwrap();
    register_local_in(&store, "coder", "pk-coder", "archived", "sid-coder", 1_000);
    store
        .upsert_status(&Status {
            pubkey: "pk-reviewer".to_string(),
            session_id: "sid-reviewer".to_string(),
            channel_h: "archived".to_string(),
            slug: "reviewer".to_string(),
            title: "done".to_string(),
            activity: String::new(),
            busy: false,
            last_seen: 1_000,
            updated_at: 1_000,
            expiration: 1_090,
        })
        .unwrap();

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    assert!(snapshot.rows.is_empty());
}

mod dormant;
mod projection;

#[test]
fn same_host_peer_is_not_remote() {
    // A sibling agent (e.g. codex@) on the SAME laptop arrives as a peer row; it
    // must NOT be tagged remote (the bug being fixed).
    let store = Store::open_memory().unwrap();
    record_peer(&store, "pk-codex", "codex", "laptop", "", false, 1_000);
    let snap = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let sib = snap
        .rows
        .iter()
        .find(|r| r.slug == "codex")
        .expect("sibling row");
    assert!(!sib.remote, "same-host peer must not be remote");
    assert_eq!(sib.host, "laptop");
}

#[test]
fn remote_peer_shows_host_and_flag() {
    let store = Store::open_memory().unwrap();
    record_peer(&store, "pk-a", "a", "tower", "", false, 1_000);
    let snap = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let peer = snap.rows.first().expect("remote peer row");
    assert!(peer.remote);
    assert_eq!(peer.host, "tower");
}
