//! Read the recent conversation from a Claude Code (or compatible) transcript
//! JSONL, so the distiller summarizes what the agent is *actually doing* from the
//! conversation — exactly like `pc` consumes `transcript_path` — rather than
//! guessing intent from isolated tool names.
//!
//! Transcript lines look like:
//!   {"type":"user","message":{"role":"user","content":"..."| [blocks]}, ...}
//!   {"type":"assistant","message":{"role":"assistant","content":[{type:text,text},{type:tool_use,name,input}]}}
//!   {"type":"response_item","payload":{"type":"message","role":"assistant","content":[{type:output_text,text}]}}
//! We extract recent user prompts + assistant text + tool uses, skipping
//! tool-result noise, and return a compact chronological snippet.

use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const TAIL_BYTES: u64 = 96 * 1024;

/// The tail of `path` split into lines, dropping the partial first line left by
/// the mid-file seek. `None` if the file is unreadable.
fn tail_lines(path: &Path) -> Option<Vec<String>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Some(lines)
}

/// A compact, chronological snippet of the last `max_msgs` user/assistant turns,
/// capped at `max_chars`. `None` if the file is unreadable/empty.
pub fn read_recent(path: &Path, max_msgs: usize, max_chars: usize) -> Option<String> {
    let lines = tail_lines(path)?;

    let mut msgs: Vec<String> = Vec::new();
    for line in &lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        // Three accepted shapes:
        //  - Claude Code: top-level `type` ("user"/"assistant"), content nested
        //    under `message.content` (string or block array).
        //  - Flat (opencode plugin, like pc): top-level `role` + `content` string.
        //  - Codex rollout JSONL: top-level `response_item`, message nested
        //    under `payload` with `input_text`/`output_text` blocks.
        let Some((role, content)) = message_record(&v) else {
            continue;
        };

        let body = extract(content, role);
        if !body.trim().is_empty() {
            let who = if role == "user" { "User" } else { "Assistant" };
            msgs.push(format!("{who}: {}", truncate(&body, 400)));
        }
    }

    if msgs.is_empty() {
        return None;
    }
    let tail: Vec<String> = msgs.iter().rev().take(max_msgs).rev().cloned().collect();
    let joined = tail.join("\n");
    Some(cap_tail(&joined, max_chars))
}

/// The text of the LAST assistant message in the transcript, capped at
/// `max_chars`. Used to auto-publish an agent's final response when its turn
/// ended without an explicit `chat write`. `None` when the transcript holds no
/// non-empty assistant text.
pub fn read_last_assistant_text(path: &Path, max_chars: usize) -> Option<String> {
    let lines = tail_lines(path)?;
    let mut last: Option<String> = None;
    for line in &lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some((role, content)) = message_record(&v) else {
            continue;
        };
        if role != "assistant" {
            continue;
        }
        let body = extract(content, role);
        if !body.trim().is_empty() {
            last = Some(body.trim().to_string());
        }
    }
    last.map(|b| truncate(&b, max_chars))
}

fn message_record(v: &Value) -> Option<(&str, Option<&Value>)> {
    if v.get("type").and_then(|x| x.as_str()) == Some("response_item") {
        let payload = v.get("payload")?;
        if payload.get("type").and_then(|x| x.as_str()) != Some("message") {
            return None;
        }
        let role = payload.get("role").and_then(|x| x.as_str())?;
        if role != "user" && role != "assistant" {
            return None;
        }
        return Some((role, payload.get("content")));
    }

    if let Some(t) = v.get("type").and_then(|x| x.as_str()) {
        if t != "user" && t != "assistant" {
            return None;
        }
        return Some((t, v.get("message").and_then(|m| m.get("content"))));
    }

    let role = v.get("role").and_then(|x| x.as_str())?;
    if role != "user" && role != "assistant" {
        return None;
    }
    Some((role, v.get("content")))
}

fn extract(content: Option<&Value>, _role: &str) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => {
            let mut parts = Vec::new();
            for b in blocks {
                // tool_use, tool_result, and others are noise for distillation.
                if matches!(
                    b.get("type").and_then(|x| x.as_str()),
                    Some("text" | "input_text" | "output_text")
                ) {
                    if let Some(t) = b.get("text").and_then(|x| x.as_str()) {
                        parts.push(t.to_string());
                    }
                }
            }
            parts.join(" ")
        }
        _ => String::new(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

/// Keep the *last* `n` chars (char-boundary safe) — the most recent context.
fn cap_tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}

#[cfg(test)]
mod tests;
