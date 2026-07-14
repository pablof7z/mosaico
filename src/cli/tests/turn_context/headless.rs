use super::{seed_channel, test_session, BACKEND};
use crate::state::Store;
use crate::turn_context::{
    assemble_turn_check, assemble_turn_check_context, assemble_turn_start,
    assemble_turn_start_context, HookContextGraphs,
};
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

/// A turn check without a preceding turn-start is still delta-only: it records
/// the baseline without injecting a synthetic mode notice.
#[test]
fn turn_check_does_not_inject_an_initial_mode_notice() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let m = Mutex::new(store);
    let ctx = assemble_turn_check_context(&m, &test_session("sess-no-rows"), "laptop", None, 200);
    assert!(
        ctx.is_none(),
        "turn check must stay delta-only; got: {ctx:?}"
    );
}

#[test]
fn persistent_context_reports_only_an_output_mode_transition() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let rec = test_session("sess-mode-transition");
    let m = Mutex::new(store);
    let graphs = HookContextGraphs::default();

    let start = assemble_turn_start(&m, &rec, BACKEND, "laptop", 0, &graphs);
    assert!(start
        .text
        .is_some_and(|text| text.contains("Headless mode is off.")));
    assert!(
        assemble_turn_check(&m, &rec, "laptop", None, 200, &graphs)
            .text
            .is_none(),
        "an unchanged mode must not produce a mid-turn injection"
    );

    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("detached.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    m.lock()
        .unwrap()
        .put_alias(
            "claude-code",
            "pty_session",
            socket_path.to_str().unwrap(),
            &rec.session_id,
            1,
        )
        .unwrap();

    let changed = assemble_turn_check(&m, &rec, "laptop", None, 201, &graphs)
        .text
        .expect("a presentation transition must be injected");
    assert!(changed.contains("Headless mode is on."), "{changed}");
    assert!(
        assemble_turn_check(&m, &rec, "laptop", None, 202, &graphs)
            .text
            .is_none(),
        "the same presentation mode must not repeat"
    );
}
