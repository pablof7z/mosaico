use super::*;
use crate::daemon::tail_event::TailEvent;

const TS: u64 = 1_700_000_000; // 2023-11-14 22:13:20 UTC  → 22:13:20 wall-clock

fn ts_str() -> String {
    let h = (TS % 86400) / 3600;
    let m = (TS % 3600) / 60;
    let s = TS % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

// ── Msg ─────────────────────────────────────────────────────────────────

#[test]
fn render_msg_no_color_no_emoji() {
    let ev = TailEvent::Msg {
        ts: TS,
        channel: "proj".into(),
        from: "claude".into(),
        from_session: Some("te-abc-111".into()),
        to: "codex".into(),
        to_session: None,
        body: "can you review the codec?".into(),
    };
    let line = render_tail_event(&ev, false, false, false, false);
    assert!(line.starts_with(&ts_str()), "should start with timestamp");
    assert!(line.contains("msg"), "should contain category");
    assert!(line.contains("claude@proj"), "should contain agent@channel");
    assert!(line.contains("->"), "ASCII arrow when no_emoji");
    assert!(line.contains("codex"), "should contain recipient");
    assert!(line.contains("review the codec"), "should contain body");
}

#[test]
fn render_msg_with_emoji() {
    let ev = TailEvent::Msg {
        ts: TS,
        channel: "proj".into(),
        from: "claude".into(),
        from_session: None,
        to: "codex".into(),
        to_session: None,
        body: "hello".into(),
    };
    let line = render_tail_event(&ev, false, true, false, false);
    assert!(line.contains("→"), "Unicode arrow when emoji enabled");
}

// ── Turn ─────────────────────────────────────────────────────────────────

#[test]
fn render_turn_working_no_color() {
    let ev = TailEvent::Turn {
        ts: TS,
        channel: "proj".into(),
        agent: "claude".into(),
        session: "te-session-1".into(),
        state: "working".into(),
        elapsed_s: None,
    };
    let line = render_tail_event(&ev, false, false, false, false);
    assert!(line.contains("turn"), "category");
    assert!(line.contains("claude@proj"), "agent@channel");
    assert!(line.contains("started working"), "state label");
    assert!(line.contains(">"), "ASCII glyph when no emoji");
}

#[test]
fn render_turn_idle_with_elapsed() {
    let ev = TailEvent::Turn {
        ts: TS,
        channel: "proj".into(),
        agent: "claude".into(),
        session: "te-session-1".into(),
        state: "idle".into(),
        elapsed_s: Some(91),
    };
    let line = render_tail_event(&ev, false, false, false, false);
    assert!(line.contains("idle"), "should contain idle label");
    assert!(line.contains("1m31s"), "should contain formatted duration");
}

// ── Join / Leave ─────────────────────────────────────────────────────────

#[test]
fn render_join_no_color() {
    let ev = TailEvent::Join {
        ts: TS,
        channel: "tenex-edge".into(),
        agent: "codex".into(),
        host: "tower".into(),
        session: "te-peer-abc".into(),
        rel_cwd: ".".into(),
    };
    let line = render_tail_event(&ev, false, false, false, false);
    assert!(line.contains("join"), "category");
    assert!(line.contains("codex@tower"), "agent@backend-label");
    assert!(line.contains("online"), "verb");
    assert!(line.contains("tenex-edge"), "channel");
}

#[test]
fn render_leave_formats_duration() {
    let ev = TailEvent::Leave {
        ts: TS,
        channel: "proj".into(),
        agent: "opencode".into(),
        host: "tower".into(),
        session: "te-peer-def".into(),
        online_s: 1020,
    };
    let line = render_tail_event(&ev, false, false, false, false);
    assert!(line.contains("leave"), "category");
    assert!(line.contains("offline"), "verb");
    assert!(line.contains("17m0s"), "duration 1020s = 17m0s");
}

// ── Sync ─────────────────────────────────────────────────────────────────

#[test]
fn render_failed_sync_includes_detail() {
    let ev = TailEvent::Sync {
        ts: TS,
        channel: "proj".into(),
        from: "tenex-edge".into(),
        to: "codex".into(),
        state: "failed".into(),
        detail: Some("session sid-1: failed to read inbox".into()),
    };
    let line = render_tail_event(&ev, false, false, false, false);
    assert!(line.contains("sync"), "category");
    assert!(line.contains("[x] failed"), "failure glyph");
    assert!(
        line.contains("session sid-1: failed to read inbox"),
        "detail"
    );
}

// ── Sess ─────────────────────────────────────────────────────────────────

#[test]
fn render_sess_start_no_color() {
    let ev = TailEvent::Sess {
        ts: TS,
        channel: "proj".into(),
        agent: "claude".into(),
        session: "te-abc-999".into(),
        state: "start".into(),
        rel_cwd: ".".into(),
    };
    let line = render_tail_event(&ev, false, false, false, false);
    assert!(line.contains("sess"), "category");
    assert!(line.contains("session start"), "state label");
}

// ── parse_since ──────────────────────────────────────────────────────────

#[test]
fn parse_since_unix_passthrough() {
    assert_eq!(parse_since("1700000000"), 1_700_000_000);
}

#[test]
fn parse_since_duration_h() {
    let now = now_secs();
    let result = parse_since("1h");
    let expected = now.saturating_sub(3600);
    // Allow ±2s for timing.
    assert!((result as i64 - expected as i64).abs() <= 2, "1h parse");
}

#[test]
fn parse_since_zero_for_garbage() {
    assert_eq!(parse_since("not-a-time"), 0);
}

#[test]
fn agent_env_prefers_active_over_fallback() {
    assert_eq!(
        select_agent_env(Some("haiku".into()), Some("developer".into())).as_deref(),
        Some("haiku")
    );
    assert_eq!(
        select_agent_env(None, Some("developer".into())).as_deref(),
        Some("developer")
    );
    assert_eq!(select_agent_env(Some(String::new()), None), None);
}
