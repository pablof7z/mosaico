//! Human render for `probe state`.

use serde_json::Value;
use std::fmt::Write as _;

fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn i64_at(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

fn bool_at(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn strs(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `probe state <surface>` — live per-surface values.
pub(super) fn render_state(v: &Value) -> String {
    let mut out = String::new();
    let surface = str_at(v, "surface");
    let _ = writeln!(out, "state {surface}  (live)");
    let empty = Vec::new();
    let rows = v.get("rows").and_then(Value::as_array).unwrap_or(&empty);
    if rows.is_empty() {
        let _ = writeln!(out, "  (none)");
        if let Some(note) = v.get("note").and_then(Value::as_str) {
            let _ = writeln!(out, "  {note}");
        }
    }
    for r in rows {
        match surface {
            "status" => render_status_row(&mut out, r),
            "hook_context" => render_hook_row(&mut out, r),
            "turn_lifecycle" => render_turn_lifecycle_row(&mut out, r),
            "cursor" => render_cursor_row(&mut out, r),
            "delivery" => render_delivery_row(&mut out, r),
            "session_start" => render_session_start_row(&mut out, r),
            "outbox" => render_outbox_row(&mut out, r),
            _ => render_resource_row(&mut out, r),
        }
    }
    out
}

fn render_delivery_row(out: &mut String, r: &Value) {
    let handle = row_handle(r, "delivery");
    let pty = str_at(r, "pty_id");
    let pty = if pty.is_empty() { "-" } else { pty };
    let _ = writeln!(
        out,
        "  {:<24} {:<20} events={:?}  pty={}  retry_after={}",
        handle,
        str_at(r, "action"),
        strs(r, "event_ids"),
        clipped(pty, 16),
        i64_at(r, "retry_after_secs"),
    );
}

fn render_status_row(out: &mut String, r: &Value) {
    let handle = row_handle(r, "status");
    let _ = writeln!(
        out,
        "  {:<24} {:<6} title={:?}  activity={:?}  channels={:?}",
        handle,
        if r.get("busy").and_then(Value::as_bool) == Some(true) {
            "busy"
        } else {
            "idle"
        },
        str_at(r, "title"),
        str_at(r, "activity"),
        strs(r, "channels"),
    );
}

fn render_hook_row(out: &mut String, r: &Value) {
    let handle = if !str_at(r, "resource_key").is_empty() {
        str_at(r, "resource_key").to_string()
    } else if str_at(r, "view_label").is_empty() {
        format!("hook/{}/view", str_at(r, "session"))
    } else {
        str_at(r, "view_label").to_string()
    };
    let _ = writeln!(
        out,
        "  {:<24} rev {}  nodes {}  renders {}",
        handle,
        i64_at(r, "revision"),
        i64_at(r, "nodes"),
        i64_at(r, "render_count"),
    );
    let causes = strs(r, "why_input_causes");
    if !causes.is_empty() {
        let _ = writeln!(out, "      caused by: {}", causes.join(", "));
    }
    let inputs = strs(r, "input_labels");
    if !inputs.is_empty() {
        let _ = writeln!(out, "      inputs:    {}", inputs.join(", "));
    }
    if let Some(text) = r.get("text").and_then(Value::as_str) {
        let first = text.lines().next().unwrap_or("");
        let _ = writeln!(out, "      text:      {first:?}");
    }
    if let Some(dump) = r.get("debug_dump").and_then(Value::as_str) {
        let _ = writeln!(out, "      dump:\n{dump}");
    }
}

fn render_turn_lifecycle_row(out: &mut String, r: &Value) {
    let handle = row_handle(r, "turn_lifecycle");
    let mode = if bool_at(r, "working") {
        "working"
    } else {
        "idle"
    };
    let transcript = str_at(r, "transcript_ref");
    let transcript = if transcript.is_empty() {
        "-"
    } else {
        transcript
    };
    let _ = writeln!(
        out,
        "  {:<24} {:<8} started={}  transcript={}",
        handle,
        mode,
        i64_at(r, "turn_started_at"),
        transcript,
    );
}

fn render_cursor_row(out: &mut String, r: &Value) {
    let handle = row_handle(r, "cursor");
    let _ = writeln!(
        out,
        "  {:<24} cursor={}  last_frame={}  delta_since={}",
        handle,
        i64_at(r, "cursor"),
        i64_at(r, "last_frame"),
        i64_at(r, "delta_since"),
    );
}

fn render_session_start_row(out: &mut String, r: &Value) {
    let handle = row_handle(r, "session_start");
    let signer = str_at(r, "signer_pubkey");
    let _ = writeln!(
        out,
        "  {:<24} {:<13} channel={}  signer={}  reassert={}",
        handle,
        str_at(r, "action"),
        str_at(r, "channel_h"),
        clipped(signer, 12),
        bool_at(r, "reassert"),
    );
    let mut intents = Vec::new();
    if bool_at(r, "has_channel_ready_intent") {
        intents.push("channel_ready");
    }
    if bool_at(r, "has_spawn_intent") {
        intents.push("spawn");
    }
    if bool_at(r, "ensure_subscription") {
        intents.push("subscription");
    }
    if bool_at(r, "replay_chat") {
        intents.push("chat_replay");
    }
    if !intents.is_empty() {
        let _ = writeln!(out, "      intents: {}", intents.join(", "));
    }
    if !str_at(r, "failure_stage").is_empty() || !str_at(r, "failure_error").is_empty() {
        let _ = writeln!(
            out,
            "      failed at {}: {}",
            str_at(r, "failure_stage"),
            str_at(r, "failure_error")
        );
    }
}

fn render_outbox_row(out: &mut String, r: &Value) {
    let handle = if str_at(r, "resource_key").is_empty() {
        format!("outbox/{}", i64_at(r, "local_id"))
    } else {
        str_at(r, "resource_key").to_string()
    };
    let source = str_at(r, "source_ref");
    let source = if source.is_empty() { "-" } else { source };
    let _ = writeln!(
        out,
        "  {:<24} {:<9} retries={:<4} event={}  source={}",
        handle,
        str_at(r, "state"),
        i64_at(r, "retries"),
        clipped(str_at(r, "event_id"), 12),
        source,
    );
    if !str_at(r, "last_error").is_empty() {
        let _ = writeln!(out, "      error: {}", str_at(r, "last_error"));
    }
}

fn render_resource_row(out: &mut String, r: &Value) {
    let _ = writeln!(
        out,
        "  {:<24} refcount {}   owners: {}",
        str_at(r, "resource_key"),
        i64_at(r, "refcount"),
        strs(r, "owners").join(", "),
    );
}

fn row_handle(r: &Value, prefix: &str) -> String {
    if str_at(r, "resource_key").is_empty() {
        format!("{}/{}", prefix, str_at(r, "session"))
    } else {
        str_at(r, "resource_key").to_string()
    }
}

fn clipped(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}
