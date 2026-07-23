use super::*;
use crate::state::{RegisterSession, Status};
use crate::who_snapshot::{load_who_snapshot, WhoSource};

/// Register a local pubkey-owned session.
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
    if store.get_channel(channel).unwrap().is_none() {
        store.upsert_channel(channel, channel, "", "", ts).unwrap();
    }
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.to_string(),
            observed_harness: "claude-code".to_string(),
            agent_slug: slug.to_string(),
            channel_h: channel.to_string(),
            child_pid: Some(42),
            transcript_path: None,
            now: ts,
        })
        .unwrap();
    store.allocate_handle(pubkey, slug, ts).unwrap();
    store
        .put_session_locator(
            "claude-code",
            crate::state::LOCATOR_NATIVE_RESUME,
            ext_id,
            pubkey,
            ts,
        )
        .unwrap();
    pubkey.to_string()
}

/// Set a session's local pre-publish draft title. Local rows fall back to this
/// when no kind:30315 has been published yet.
fn seed_draft_title(store: &Store, session_id: &str, title: &str, _ts: u64) {
    store.set_session_title(session_id, title).unwrap();
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
    if store.get_channel("proj").unwrap().is_none() {
        store.upsert_channel("proj", "proj", "", "", ts).unwrap();
    }
    store
        .upsert_profile(pubkey, slug, slug, host, false, 1)
        .unwrap();
    store
        .upsert_status(&Status {
            pubkey: pubkey.to_string(),
            channel_h: "proj".to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            activity: String::new(),
            state: if busy {
                crate::session_state::SessionState::Working
            } else {
                crate::session_state::SessionState::Idle
            },
            state_since: ts,
            last_seen: ts,
            updated_at: ts,
            expiration: ts + 90,
        })
        .unwrap();
}

/// Declare `pubkey` as a key this daemon signs as, so a relay echo of our own
/// status is dropped from the peer set.
fn own_identity(store: &Store, pubkey: &str, slug: &str) {
    let _ = slug;
    store.bind_session_signer(pubkey, "test-salt").unwrap();
}

#[test]
fn who_snapshot_merges_local_and_peer_sessions() {
    let store = Store::open_memory().unwrap();
    // Local coder is a hosted session; pk-coder is one of our signing keys.
    own_identity(&store, "pk-coder", "coder");
    let _coder_sid = register_local(&store, "coder", "pk-coder", "sid-coder", 1_000);
    let coder_handle = store
        .handle_for_pubkey("pk-coder")
        .unwrap()
        .expect("derived session public handle");
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
    let id_a = register_local(&store, "claude", "pk-claude-a", "sid-a", 1_000);
    let id_b = register_local(&store, "claude", "pk-claude-b", "sid-b", 1_000);
    seed_draft_title(&store, &id_a, "reading files", 1_000);
    seed_draft_title(&store, &id_b, "running tests", 1_000);

    let snapshot = load_who_snapshot(&store, Some("proj"), 1_000, "laptop").unwrap();
    let row_a = snapshot
        .rows
        .iter()
        .find(|r| r.pubkey == id_a)
        .expect("session-a row");
    let row_b = snapshot
        .rows
        .iter()
        .find(|r| r.pubkey == id_b)
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
            channel_h: "archived".to_string(),
            slug: "reviewer".to_string(),
            title: "done".to_string(),
            activity: String::new(),
            state: crate::session_state::SessionState::Idle,
            state_since: 1_000,
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
