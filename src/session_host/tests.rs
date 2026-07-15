use super::registry::{build_headless_command, headless_shape_for_harness, HeadlessShape};

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn headless_shape_is_keyed_by_harness() {
    assert_eq!(
        headless_shape_for_harness(crate::session::Harness::ClaudeCode),
        Some(HeadlessShape::ClaudePrint)
    );
    assert_eq!(
        headless_shape_for_harness(crate::session::Harness::Codex),
        Some(HeadlessShape::CodexExec)
    );
    assert_eq!(
        headless_shape_for_harness(crate::session::Harness::Opencode),
        Some(HeadlessShape::OpencodeRun)
    );
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
        pubkey: "pk-target".into(),
        runtime_generation: 1,
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
        work_topic: String::new(),
        work_topic_set_at: 0,
        seen_cursor: 0,
        title: String::new(),
        activity: String::new(),
        distill_fail_streak: 0,
        distill_notice_at: 0,
        explicit_chat_published_at: 0,
    }
}

#[test]
fn pending_message_prompt_contains_the_actual_message_body() {
    let rec = sample_session();
    // Renderer shows the short sender pubkey.
    let row = crate::state::InboxRow {
        event_id: "abcdef123456".into(),
        target_pubkey: rec.pubkey.clone(),
        state: "pending".into(),
        from_pubkey: "pk-sender".into(),
        channel_h: "proj".into(),
        body: "please review the PTY delivery path".into(),
        created_at: 100,
        delivered_at: 0,
    };

    // No whitelist → the sender is treated as another agent. With no cached slug
    // the name falls back to the short sender pubkey ("pk-sende"), and with no
    // channel metadata the source room is still the workspace's general channel.
    let store = crate::state::Store::open_memory().unwrap();
    let prompt = crate::injection::render_terminal_mention(&store, &[row], &[], 120).unwrap();

    assert_eq!(
        prompt,
        "<mosaico>\n\
         \u{20}\u{20}<channel ref=\"proj\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@pk-sende\" id=\"abcdef\">please review the PTY delivery path</message>\n\
         \u{20}\u{20}</channel>\n\
         \n\
         \u{20}\u{20}Reply via: `mosaico channel reply abcdef --message \"hello world\"`\n\
         \u{20}\u{20}Attachments: add `--attach label=/path/to/file` and reference `[label]` in the message.\n\
         </mosaico>"
    );
}

#[test]
fn whitelisted_human_mention_renders_bare_with_provenance() {
    let rec = sample_session();
    let row = crate::state::InboxRow {
        event_id: "ev-human".into(),
        target_pubkey: rec.pubkey.clone(),
        state: "pending".into(),
        from_pubkey: "human-pk".into(),
        channel_h: "channel-writer-test".into(),
        body: "@developer hey there".into(),
        created_at: 100,
        delivered_at: 0,
    };
    let store = crate::state::Store::open_memory().unwrap();
    store
        .upsert_channel("mosaico", "mosaico", "", "", 1)
        .unwrap();
    store
        .upsert_channel("channel-writer-test", "writer-test", "", "mosaico", 100)
        .unwrap();
    // Sender is whitelisted, but the injected line still carries the source room.
    let prompt =
        crate::injection::render_terminal_mention(&store, &[row], &["human-pk".into()], 120)
            .unwrap();
    assert_eq!(
        prompt,
        "<mosaico>\n\
         \u{20}\u{20}<channel ref=\"mosaico.writer-test\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@human-pk\" id=\"ev-hum\">@developer hey there</message>\n\
         \u{20}\u{20}</channel>\n\
         \n\
         \u{20}\u{20}Reply via: `mosaico channel reply ev-hum --message \"hello world\"`\n\
         \u{20}\u{20}Attachments: add `--attach label=/path/to/file` and reference `[label]` in the message.\n\
         </mosaico>"
    );
}
