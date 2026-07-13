use super::{seed_channel, test_session, BACKEND};
use crate::state::Store;
use crate::turn_context::{assemble_turn_check_context, assemble_turn_start_context};
use std::sync::Mutex;

/// A fresh hook graph introduces the output mode even for a non-first turn.
#[test]
fn turn_start_introduces_headless_mode_for_an_existing_session() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let mut rec = test_session("sess-freeze-2");
    rec.seen_cursor = crate::util::now_secs();
    let m = Mutex::new(store);

    let ctx = assemble_turn_start_context(&m, &rec, BACKEND, "laptop", 42);
    assert!(
        ctx.as_ref()
            .is_some_and(|text| text.contains("Headless mode is off.")),
        "fresh context must introduce output mode; got: {ctx:?}"
    );
}

/// A direct session is headed while idle steering remains unavailable.
#[test]
fn direct_output_and_idle_reachability_are_separate() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let rec = test_session("sess-no-pty");
    let m = Mutex::new(store);

    let text = assemble_turn_start_context(&m, &rec, BACKEND, "laptop", 0).unwrap();
    assert!(text.contains("cannot receive automatic steering while idle"));
    assert!(text.contains("Headless mode is off."));
}

/// A live but detached PTY is headless while its delivery endpoint remains live.
#[test]
fn detached_pty_is_headless_and_steerable() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let rec = test_session("sess-with-pty");
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("live.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    store
        .put_alias(
            "claude-code",
            "pty_session",
            socket_path.to_str().unwrap(),
            &rec.session_id,
            1,
        )
        .unwrap();
    let m = Mutex::new(store);

    let text = assemble_turn_start_context(&m, &rec, BACKEND, "laptop", 0).unwrap();
    assert!(!text.contains("cannot receive automatic steering while idle"));
    assert!(text.contains("Headless mode is on."));
}

/// A fresh turn-check graph introduces the output mode before any other delta.
#[test]
fn turn_check_introduces_headless_mode_when_nothing_else_is_due() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let m = Mutex::new(store);
    let ctx = assemble_turn_check_context(&m, &test_session("sess-no-rows"), "laptop", None, 200);
    assert!(
        ctx.as_ref()
            .is_some_and(|text| text.contains("Headless mode is off.")),
        "fresh turn check must introduce output mode; got: {ctx:?}"
    );
}
