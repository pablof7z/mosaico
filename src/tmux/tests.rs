use super::registry::{build_resume_command, resume_shape_for_bin, ResumeShape};

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
fn default_tmux_statusline_uses_harness_subcommand() {
    let cmd = super::launch::default_statusline_cmd("/tmp/tenex-edge");
    assert!(cmd.contains("/tmp/tenex-edge harness statusline --tmux"));
    assert!(!cmd.contains("/tmp/tenex-edge statusline --tmux"));
    assert!(cmd.contains("@te_session"));
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
        body: "please review the tmux delivery path".into(),
        created_at: 100,
        delivered_at: 0,
    };

    // No whitelist → the sender is treated as another agent, so the paste form is
    // the framed `[tenex-edge mention] <@name> body` line. With no cached slug the
    // name falls back to the short sender pubkey ("pk-sende").
    let store = crate::state::Store::open_memory().unwrap();
    let prompt = crate::injection::render_tmux_mention(&store, &[row], &[], 120).unwrap();

    assert_eq!(
        prompt,
        "[tenex-edge mention] <@pk-sende> please review the tmux delivery path\n\
         [reply via `tenex-edge chat write --message \"...\"` — replies do not auto-publish]"
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
        channel_h: "proj".into(),
        body: "@developer hey there".into(),
        created_at: 100,
        delivered_at: 0,
    };
    let store = crate::state::Store::open_memory().unwrap();
    // Sender is whitelisted → minimal provenance, no `[tenex-edge mention]` frame.
    let prompt =
        crate::injection::render_tmux_mention(&store, &[row], &["human-pk".into()], 120).unwrap();
    assert_eq!(
        prompt,
        "<@human-pk> @developer hey there\n\
         [reply via `tenex-edge chat write --message \"...\"` — replies do not auto-publish]"
    );
}
