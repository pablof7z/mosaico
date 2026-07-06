use super::*;
use crate::fabric_context::{capture_inputs, render_fabric_context, FabricContextInput};
use crate::state::{RegisterSession, RelayEvent, Session, Status, Store};

const SELF_PK: &str = "self-pubkey";
const OTHER_PK: &str = "other-pubkey";

fn seed_store() -> Store {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("root", "main", "Root room", "", 1)
        .unwrap();
    store
        .upsert_channel("task", "task", "Task room", "root", 1)
        .unwrap();
    store
        .replace_channel_members("root", &[SELF_PK.into(), OTHER_PK.into()], 1)
        .unwrap();
    store
        .replace_channel_members("task", &[SELF_PK.into(), OTHER_PK.into()], 1)
        .unwrap();
    store
        .upsert_profile(SELF_PK, "coder", "coder", "laptop", false, 1)
        .unwrap();
    store
        .upsert_profile(OTHER_PK, "reviewer", "reviewer", "laptop", false, 1)
        .unwrap();
    store
}

fn session(store: &Store) -> Session {
    let id = store
        .register_session(&RegisterSession {
            harness: "test".into(),
            external_id_kind: "test".into(),
            external_id: "sess".into(),
            agent_pubkey: SELF_PK.into(),
            agent_slug: "coder".into(),
            channel_h: "root".into(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 10,
        })
        .unwrap();
    store.join_session_channel(&id, "task", 20).unwrap();
    store.get_session(&id).unwrap().unwrap()
}

fn chat(store: &Store, id: &str, channel: &str, at: u64, body: &str, tags_json: &str) {
    store
        .insert_event(&RelayEvent {
            id: id.into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: OTHER_PK.into(),
            created_at: at,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: body.into(),
            tags_json: tags_json.into(),
        })
        .unwrap();
}

fn status(store: &Store, channel: &str, busy: bool, activity: &str, updated_at: u64) {
    store
        .upsert_status(&Status {
            pubkey: OTHER_PK.into(),
            session_id: "peer-sess".into(),
            channel_h: channel.into(),
            slug: "reviewer".into(),
            title: String::new(),
            activity: activity.into(),
            busy,
            last_seen: updated_at,
            updated_at,
            expiration: 10_000,
        })
        .unwrap();
}

fn fabric_input<'a>(
    rec: &'a Session,
    scope: &'a str,
    cursor: u64,
    now: u64,
    force: bool,
) -> FabricContextInput<'a> {
    FabricContextInput {
        session: Some(rec),
        scope,
        cursor,
        now,
        self_slug: "coder",
        self_pubkey: SELF_PK,
        local_host: "laptop",
        forced_messages: &[],
        warnings: &[],
        force,
    }
}

/// HEADLINE: same inputs + same cursor + same now → byte-identical snapshot AND
/// identical receipt. Two independent graphs must replay to the same product.
#[test]
fn determinism_and_replay() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "m1", "root", 900, "hello world", "[]");
    status(&store, "root", true, "compiling", 500);
    let input = fabric_input(&rec, "root", 0, 1_000, false);
    let captured = capture_inputs(&store, &input);

    let mut a = HookContextReconciler::new();
    let out_a = a
        .render_context("sess", "turn_start", 0, 1_000, captured.clone())
        .unwrap();
    a.assert_oracle().unwrap();

    let mut b = HookContextReconciler::new();
    let out_b = b
        .render_context("sess", "turn_start", 0, 1_000, captured)
        .unwrap();
    b.assert_oracle().unwrap();

    assert_eq!(out_a.text, out_b.text, "snapshot bytes must be identical");
    assert!(out_a.text.as_ref().unwrap().contains("<tenex-edge>"));
    assert_eq!(out_a.receipt, out_b.receipt, "receipts must be identical");
    assert_eq!(out_a.receipt.frame, FrameKind::Baseline);
}

/// `cursor == 0` → full `<members>`; `cursor > 0` → delta `<recent-presence>`
/// only. The shape flip is attributable to the `cursor` input via the frame's
/// own dependency trace — not a re-derivation.
#[test]
fn cursor_drives_shape_and_is_attributed() {
    let store = seed_store();
    let rec = session(&store);
    // A status updated AFTER the delta cursor so the delta render has presence.
    status(&store, "root", true, "compiling", 150);

    let mut r = HookContextReconciler::new();
    // Baseline: full render at cursor 0.
    let full_input = capture_inputs(&store, &fabric_input(&rec, "root", 0, 300, false));
    let full = r
        .render_context("sess", "turn_start", 0, 300, full_input)
        .unwrap();
    r.assert_oracle().unwrap();
    let full_text = full.text.unwrap();
    assert!(full_text.contains("<members>"), "cursor==0 renders members");
    assert!(
        !full_text.contains("<recent-presence>"),
        "cursor==0 hides the delta presence block"
    );

    // Delta: same store, cursor advanced → members drop, presence appears.
    let delta_input = capture_inputs(&store, &fabric_input(&rec, "root", 100, 300, false));
    let delta = r
        .render_context("sess", "turn_check", 100, 300, delta_input)
        .unwrap();
    r.assert_oracle().unwrap();
    let delta_text = delta.text.unwrap();
    assert!(
        !delta_text.contains("<members>"),
        "cursor>0 drops the full member roster"
    );
    assert!(
        delta_text.contains("<recent-presence>"),
        "cursor>0 renders the delta presence block: {delta_text}"
    );

    assert_eq!(delta.receipt.shape, Shape::Delta);
    assert!(
        delta.receipt.input_causes.iter().any(|c| c == "cursor"),
        "shape flip attributed to the cursor input: {:?}",
        delta.receipt.input_causes
    );
    let cursor_id = r.cursor_input().unwrap();
    assert!(
        r.why_view_causes().contains(&cursor_id),
        "the view change is caused by the cursor input"
    );
}

/// A presence row's busy/activity change moves the snapshot, and the frame is
/// attributed to the `presence` input — the "why is @X shown as working" receipt,
/// matching the actual rendered member line.
#[test]
fn presence_change_attributed_to_presence_input() {
    let store = seed_store();
    let rec = session(&store);
    status(&store, "root", false, "", 50); // idle

    let mut r = HookContextReconciler::new();
    let base = capture_inputs(&store, &fabric_input(&rec, "root", 0, 300, true));
    let first = r
        .render_context("sess", "turn_start", 0, 300, base)
        .unwrap();
    r.assert_oracle().unwrap();
    let first_text = first.text.unwrap();
    assert!(
        first_text.contains("status=\"idle\""),
        "reviewer starts idle: {first_text}"
    );

    // @reviewer starts working on something.
    status(&store, "root", true, "refactoring", 60);
    let changed = capture_inputs(&store, &fabric_input(&rec, "root", 0, 300, true));
    let out = r
        .render_context("sess", "turn_start", 0, 300, changed)
        .unwrap();
    r.assert_oracle().unwrap();
    let text = out.text.unwrap();

    assert!(
        text.contains("status=\"refactoring\""),
        "member line reflects the new activity: {text}"
    );
    assert_eq!(out.receipt.frame, FrameKind::Delta);
    assert!(
        out.receipt.input_causes.iter().any(|c| c == "presence"),
        "the working-status change is attributed to the presence input: {:?}",
        out.receipt.input_causes
    );
    let presence_id = r.presence_input().unwrap();
    assert!(
        r.why_view_causes().contains(&presence_id),
        "the view change is caused by the presence input"
    );
}

/// Regression: the produced snapshot equals what the OLD `build_view`
/// (`render_fabric_context`) produced for the same state — for BOTH the full and
/// delta shapes. The graph derivation is a faithful, byte-exact port.
#[test]
fn equivalence_with_legacy_build_view() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "m-old", "root", 100, "old root note", "[]");
    chat(&store, "m-new", "task", 220, "new task note", "[]");
    let tags = format!("[[\"p\",\"{SELF_PK}\"]]");
    chat(&store, "m-mention", "root", 240, "ping for you", &tags);
    status(&store, "root", true, "compiling", 150);
    status(&store, "task", false, "", 210);

    for (cursor, now) in [(0u64, 300u64), (200u64, 300u64)] {
        let input = fabric_input(&rec, "root", cursor, now, true);
        let oracle = render_fabric_context(&store, fabric_input(&rec, "root", cursor, now, true));
        let captured = capture_inputs(&store, &input);
        let mut r = HookContextReconciler::new();
        let out = r
            .render_context("sess", "turn_start", cursor as i64, now as i64, captured)
            .unwrap();
        r.assert_oracle().unwrap();
        assert_eq!(
            out.text, oracle,
            "reconciler snapshot must match legacy build_view at cursor={cursor}"
        );
    }
}

/// An unchanged re-commit emits no frame, yet the cached view still yields the
/// exact same bytes and a receipt marked `Unchanged`.
#[test]
fn unchanged_recommit_reuses_cached_view() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "m1", "root", 900, "hello", "[]");

    let mut r = HookContextReconciler::new();
    let input = fabric_input(&rec, "root", 0, 1_000, false);
    let first = r
        .render_context(
            "sess",
            "turn_start",
            0,
            1_000,
            capture_inputs(&store, &input),
        )
        .unwrap();
    r.assert_oracle().unwrap();
    let second = r
        .render_context(
            "sess",
            "turn_start",
            0,
            1_000,
            capture_inputs(&store, &input),
        )
        .unwrap();
    r.assert_oracle().unwrap();

    assert_eq!(
        first.text, second.text,
        "cached view replays identical bytes"
    );
    assert_eq!(second.receipt.frame, FrameKind::Unchanged);
}
