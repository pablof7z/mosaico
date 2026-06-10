//! Read the recent conversation from a Claude Code (or compatible) transcript
//! JSONL, so the distiller summarizes what the agent is *actually doing* from the
//! conversation — exactly like `pc` consumes `transcript_path` — rather than
//! guessing intent from isolated tool names.
//!
//! Transcript lines look like:
//!   {"type":"user","message":{"role":"user","content":"..."| [blocks]}, ...}
//!   {"type":"assistant","message":{"role":"assistant","content":[{type:text,text},{type:tool_use,name,input}]}}
//! We extract recent user prompts + assistant text + tool uses, skipping
//! tool-result noise, and return a compact chronological snippet.

use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const TAIL_BYTES: u64 = 96 * 1024;

/// The text content of the last assistant message in the transcript (text blocks
/// only — tool uses are excluded). Used to populate `TurnReply` body at stop time.
/// Returns `None` if the file is unreadable or no assistant text is found.
pub fn read_last_assistant_text(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);

    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    let mut last_text: Option<String> = None;
    for line in lines {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let (role, content) = if let Some(t) = v.get("type").and_then(|x| x.as_str()) {
            if t != "assistant" {
                continue;
            }
            (t, v.get("message").and_then(|m| m.get("content")))
        } else if let Some(r) = v.get("role").and_then(|x| x.as_str()) {
            if r != "assistant" {
                continue;
            }
            (r, v.get("content"))
        } else {
            continue;
        };
        let text = extract_text_only(content);
        if !text.trim().is_empty() {
            last_text = Some(text.trim().to_string());
        }
        let _ = role;
    }
    last_text
}

/// Extract only text content blocks from an assistant message (no tool-use lines).
fn extract_text_only(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|x| x.as_str()) == Some("text") {
                    b.get("text").and_then(|x| x.as_str()).map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// A compact, chronological snippet of the last `max_msgs` user/assistant turns,
/// capped at `max_chars`. `None` if the file is unreadable/empty.
pub fn read_recent(path: &Path, max_msgs: usize, max_chars: usize) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);

    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // drop the partial first line from the mid-file seek
    }

    let mut msgs: Vec<String> = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        // Two accepted shapes:
        //  - Claude Code: top-level `type` ("user"/"assistant"), content nested
        //    under `message.content` (string or block array).
        //  - Flat (opencode plugin, like pc): top-level `role` + `content` string.
        let (role, content) = if let Some(t) = v.get("type").and_then(|x| x.as_str()) {
            if t != "user" && t != "assistant" {
                continue;
            }
            (t, v.get("message").and_then(|m| m.get("content")))
        } else if let Some(r) = v.get("role").and_then(|x| x.as_str()) {
            if r != "user" && r != "assistant" {
                continue;
            }
            (r, v.get("content"))
        } else {
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

fn extract(content: Option<&Value>, role: &str) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => {
            let mut parts = Vec::new();
            for b in blocks {
                match b.get("type").and_then(|x| x.as_str()) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(|x| x.as_str()) {
                            parts.push(t.to_string());
                        }
                    }
                    Some("tool_use") if role == "assistant" => {
                        let name = b.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                        let hint = b
                            .get("input")
                            .and_then(|i| {
                                i.get("file_path")
                                    .or_else(|| i.get("command"))
                                    .or_else(|| i.get("pattern"))
                                    .or_else(|| i.get("path"))
                                    .and_then(|x| x.as_str())
                            })
                            .unwrap_or("");
                        parts.push(
                            format!("[uses {name} {}]", truncate(hint, 80))
                                .trim()
                                .to_string(),
                        );
                    }
                    // tool_result and others are noise for "what is the agent doing".
                    _ => {}
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
mod tests {
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
        assert!(out.contains("[uses Edit src/auth.rs]"), "got: {out}");
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
    fn missing_file_is_none() {
        assert!(read_recent(Path::new("/no/such/transcript.jsonl"), 10, 1000).is_none());
    }
}
