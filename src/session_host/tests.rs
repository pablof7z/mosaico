use super::registry::{
    build_headless_command, build_resume_command, headless_shape_for_bin, resume_shape_for_bin,
    HeadlessShape, ResumeShape,
};

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn append_flag_preserves_user_launch_flags() {
    let base = cmd(&["claude", "--dangerously-skip-permissions"]);
    let got = build_resume_command(&base, ResumeShape::AppendFlag("--resume"), "abc-123");
    assert_eq!(
        got,
        cmd(&[
            "claude",
            "--dangerously-skip-permissions",
            "--resume",
            "abc-123"
        ])
    );
}

#[test]
fn append_flag_bare_command() {
    let got = build_resume_command(
        &cmd(&["opencode"]),
        ResumeShape::AppendFlag("--session"),
        "ses_x",
    );
    assert_eq!(got, cmd(&["opencode", "--session", "ses_x"]));
}

#[test]
fn subcommand_inserts_after_binary_and_keeps_flags() {
    let base = cmd(&["codex", "--dangerously-bypass-approvals-and-sandbox"]);
    let got = build_resume_command(&base, ResumeShape::Subcommand("resume"), "uuid-9");
    assert_eq!(
        got,
        cmd(&[
            "codex",
            "resume",
            "uuid-9",
            "--dangerously-bypass-approvals-and-sandbox"
        ])
    );
}

#[test]
fn subcommand_bare_command() {
    let got = build_resume_command(
        &cmd(&["codex"]),
        ResumeShape::Subcommand("resume"),
        "uuid-9",
    );
    assert_eq!(got, cmd(&["codex", "resume", "uuid-9"]));
}

#[test]
fn shape_is_keyed_by_binary_not_slug() {
    assert!(matches!(
        resume_shape_for_bin("claude"),
        Some(ResumeShape::AppendFlag("--resume"))
    ));
    assert!(matches!(
        resume_shape_for_bin("codex"),
        Some(ResumeShape::Subcommand("resume"))
    ));
    assert!(matches!(
        resume_shape_for_bin("opencode"),
        Some(ResumeShape::AppendFlag("--session"))
    ));
    assert!(matches!(
        resume_shape_for_bin("/opt/homebrew/bin/claude"),
        Some(ResumeShape::AppendFlag("--resume"))
    ));
    assert!(resume_shape_for_bin("npx").is_none());
}

#[test]
fn headless_shape_is_keyed_by_binary() {
    assert!(matches!(
        headless_shape_for_bin("claude"),
        Some(HeadlessShape::ClaudePrint)
    ));
    assert!(matches!(
        headless_shape_for_bin("/opt/homebrew/bin/codex"),
        Some(HeadlessShape::CodexExec)
    ));
    assert!(matches!(
        headless_shape_for_bin("opencode"),
        Some(HeadlessShape::OpencodeRun)
    ));
}

#[test]
fn claude_headless_fresh_preserves_flags_and_adds_print_prompt() {
    let base = cmd(&["claude", "--dangerously-skip-permissions"]);
    let got = build_headless_command(
        &base,
        HeadlessShape::ClaudePrint,
        None,
        Some("00000000-0000-4000-8000-000000000001"),
        "ship it",
    );
    assert_eq!(
        got,
        cmd(&[
            "claude",
            "--dangerously-skip-permissions",
            "--session-id",
            "00000000-0000-4000-8000-000000000001",
            "-p",
            "ship it"
        ])
    );
}

#[test]
fn claude_headless_resume_adds_resume_before_prompt() {
    let base = cmd(&["claude", "--model", "sonnet"]);
    let got = build_headless_command(
        &base,
        HeadlessShape::ClaudePrint,
        Some("claude-session"),
        None,
        "follow up",
    );
    assert_eq!(
        got,
        cmd(&[
            "claude",
            "--model",
            "sonnet",
            "-p",
            "--resume",
            "claude-session",
            "follow up"
        ])
    );
}

#[test]
fn codex_headless_fresh_inserts_exec_after_binary() {
    let base = cmd(&["codex", "--dangerously-bypass-approvals-and-sandbox"]);
    let got = build_headless_command(&base, HeadlessShape::CodexExec, None, None, "ship it");
    assert_eq!(
        got,
        cmd(&[
            "codex",
            "exec",
            "--json",
            "--dangerously-bypass-approvals-and-sandbox",
            "ship it"
        ])
    );
}

#[test]
fn codex_headless_resume_uses_exec_resume() {
    let base = cmd(&["codex", "-m", "gpt-5", "--profile", "planner"]);
    let got = build_headless_command(
        &base,
        HeadlessShape::CodexExec,
        Some("codex-session"),
        None,
        "follow up",
    );
    assert_eq!(
        got,
        cmd(&[
            "codex",
            "exec",
            "--json",
            "-m",
            "gpt-5",
            "--profile",
            "planner",
            "resume",
            "codex-session",
            "follow up"
        ])
    );
}

#[test]
fn opencode_headless_fresh_inserts_run_after_binary() {
    let base = cmd(&["opencode", "--agent", "build"]);
    // A fresh run has no forced id; opencode mints its own, recovered from the log.
    let got = build_headless_command(&base, HeadlessShape::OpencodeRun, None, None, "ship it");
    assert_eq!(
        got,
        cmd(&["opencode", "run", "--format", "json", "--agent", "build", "ship it"])
    );
}

#[test]
fn opencode_headless_resume_uses_session_flag() {
    let base = cmd(&["opencode"]);
    let got = build_headless_command(
        &base,
        HeadlessShape::OpencodeRun,
        Some("ses_0bf752c68ffeZIy7EBgv55kExz"),
        None,
        "follow up",
    );
    assert_eq!(
        got,
        cmd(&[
            "opencode",
            "run",
            "--format",
            "json",
            "--session",
            "ses_0bf752c68ffeZIy7EBgv55kExz",
            "follow up"
        ])
    );
}

fn sample_session() -> crate::state::Session {
    crate::state::Session {
        session_id: "sess-target".into(),
        agent_pubkey: "pk-target".into(),
        agent_slug: "claude".into(),
        channel_h: "proj".into(),
        harness: "claude".into(),
        child_pid: None,
        transcript_path: None,
        alive: true,
        created_at: 1000,
        last_seen: 0,
        working: false,
        turn_started_at: 0,
        last_distill_at: 0,
        seen_cursor: 0,
        title: String::new(),
        activity: String::new(),
        resume_id: String::new(),
        distill_fail_streak: 0,
        distill_notice_at: 0,
    }
}

#[test]
fn pending_message_prompt_contains_the_actual_message_body() {
    let rec = sample_session();
    // Slug is intentionally no longer carried on the inbox row; the renderer
    // shows the short sender pubkey instead.
    let row = crate::state::InboxRow {
        event_id: "abcdef123456".into(),
        target_session: rec.session_id.clone(),
        state: "pending".into(),
        from_pubkey: "pk-sender".into(),
        channel_h: "proj".into(),
        body: "please review the PTY delivery path".into(),
        created_at: 100,
        delivered_at: 0,
    };

    // No whitelist → the sender is treated as another agent. With no cached slug
    // the name falls back to the short sender pubkey ("pk-sende"), and with no
    // channel metadata the source room falls back to the raw h-tag.
    let store = crate::state::Store::open_memory().unwrap();
    let prompt = crate::injection::render_terminal_mention(&store, &[row], &[], 120).unwrap();

    assert_eq!(
        prompt,
         "<tenex-edge>\n\
         \u{20}\u{20}<channel ref=\"proj\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@pk-sende\" id=\"abcdef\">please review the PTY delivery path</message>\n\
         \u{20}\u{20}</channel>\n\
         \n\
         \u{20}\u{20}Reply via: `tenex-edge channel reply abcdef --message \"hello world\"`\n\
         </tenex-edge>"
    );
}

#[test]
fn whitelisted_human_mention_renders_bare_with_provenance() {
    let rec = sample_session();
    let row = crate::state::InboxRow {
        event_id: "ev-human".into(),
        target_session: rec.session_id.clone(),
        state: "pending".into(),
        from_pubkey: "human-pk".into(),
        channel_h: "channel-writer-test".into(),
        body: "@developer hey there".into(),
        created_at: 100,
        delivered_at: 0,
    };
    let store = crate::state::Store::open_memory().unwrap();
    store
        .upsert_channel("tenex-edge", "tenex-edge", "", "", 1)
        .unwrap();
    store
        .upsert_channel("channel-writer-test", "writer-test", "", "tenex-edge", 100)
        .unwrap();
    // Sender is whitelisted, but the injected line still carries the source room.
    let prompt =
        crate::injection::render_terminal_mention(&store, &[row], &["human-pk".into()], 120)
            .unwrap();
    assert_eq!(
        prompt,
         "<tenex-edge>\n\
         \u{20}\u{20}<channel ref=\"tenex-edge.writer-test\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@human-pk\" id=\"ev-hum\">@developer hey there</message>\n\
         \u{20}\u{20}</channel>\n\
         \n\
         \u{20}\u{20}Reply via: `tenex-edge channel reply ev-hum --message \"hello world\"`\n\
         </tenex-edge>"
    );
}
