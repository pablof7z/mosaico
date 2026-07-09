use super::*;
use std::io::Write;

#[test]
fn extracts_recent_turns_skipping_tool_results() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.jsonl");
    let mut f = File::create(&p).unwrap();
    // user prompt (string), assistant text+tool_use, user tool_result (noise)
    writeln!(
        f,
        r#"{{"type":"user","message":{{"role":"user","content":"fix the auth bug"}}}}"#
    )
    .unwrap();
    writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"Looking at the login flow"}},{{"type":"tool_use","name":"Edit","input":{{"file_path":"src/auth.rs"}}}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":"ok"}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"attachment","foo":1}}"#).unwrap();

    let out = read_recent(&p, 10, 5000).unwrap();
    assert!(out.contains("User: fix the auth bug"), "got: {out}");
    assert!(
        out.contains("Assistant: Looking at the login flow"),
        "got: {out}"
    );
    assert!(
        !out.contains("[uses Edit"),
        "tool_use should be stripped: {out}"
    );
    assert!(
        !out.contains("tool_result"),
        "tool results should be skipped: {out}"
    );
}

#[test]
fn extracts_flat_role_content_shape() {
    // The opencode plugin (like pc) writes flat {"role","content"} lines.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("flat.jsonl");
    let mut f = File::create(&p).unwrap();
    writeln!(
        f,
        r#"{{"role":"user","content":"the rate limiter drops valid requests under load"}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"role":"assistant","content":"Let me check the token-bucket refill interval"}}"#
    )
    .unwrap();
    writeln!(f, r#"{{"role":"tool","content":"noise"}}"#).unwrap();

    let out = read_recent(&p, 10, 5000).unwrap();
    assert!(
        out.contains("User: the rate limiter drops valid requests under load"),
        "got: {out}"
    );
    assert!(
        out.contains("Assistant: Let me check the token-bucket refill interval"),
        "got: {out}"
    );
    assert!(
        !out.contains("noise"),
        "non user/assistant roles should be skipped: {out}"
    );
}

#[test]
fn extracts_codex_rollout_response_items() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("codex.jsonl");
    let mut f = File::create(&p).unwrap();
    writeln!(
        f,
        r#"{{"type":"response_item","payload":{{"type":"message","role":"developer","content":[{{"type":"input_text","text":"policy noise"}}]}}}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"fix empty distillations"}}]}}}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"type":"response_item","payload":{{"type":"function_call","name":"exec_command","arguments":"{{}}"}}}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"type":"response_item","payload":{{"type":"function_call_output","output":"large tool result"}}}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"Tracing the transcript parser"}}]}}}}"#
    )
    .unwrap();

    let out = read_recent(&p, 10, 5000).unwrap();
    assert!(out.contains("User: fix empty distillations"), "got: {out}");
    assert!(
        out.contains("Assistant: Tracing the transcript parser"),
        "got: {out}"
    );
    assert!(
        !out.contains("policy noise"),
        "developer messages are noise: {out}"
    );
    assert!(
        !out.contains("large tool result"),
        "tool output should be skipped: {out}"
    );
}

#[test]
fn missing_file_is_none() {
    assert!(read_recent(Path::new("/no/such/transcript.jsonl"), 10, 1000).is_none());
}

#[test]
fn last_assistant_text_returns_final_assistant_message() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.jsonl");
    let mut f = File::create(&p).unwrap();
    writeln!(
        f,
        r#"{{"type":"user","message":{{"role":"user","content":"summarize the outage"}}}}"#
    )
    .unwrap();
    writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"first pass"}},{{"type":"tool_use","name":"Read","input":{{}}}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":"noise"}}]}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"Root cause was a retry storm; backoff added."}}]}}}}"#).unwrap();

    let out = read_last_assistant_text(&p, 5000).unwrap();
    assert_eq!(out, "Root cause was a retry storm; backoff added.");
}

#[test]
fn last_assistant_text_none_without_assistant_output() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("only-user.jsonl");
    let mut f = File::create(&p).unwrap();
    writeln!(
        f,
        r#"{{"type":"user","message":{{"role":"user","content":"hello"}}}}"#
    )
    .unwrap();
    // An assistant turn that is nothing but a tool call has no text to publish.
    writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Bash","input":{{}}}}]}}}}"#).unwrap();

    assert!(read_last_assistant_text(&p, 5000).is_none());
}
