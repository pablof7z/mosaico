use super::data::{DebugKind, SessionPane};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn cycle_filter(current: Option<&str>, values: &[String]) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let Some(cur) = current else {
        return values.first().cloned();
    };
    let next = values
        .iter()
        .position(|v| v == cur)
        .map(|i| i + 1)
        .unwrap_or(0);
    values.get(next).cloned()
}

pub(super) fn tail_read(path: &std::path::Path, max_bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let partial = len > max_bytes;
    if partial && f.seek(SeekFrom::Start(len - max_bytes)).is_err() {
        return String::new();
    }
    let mut buf = String::new();
    let _ = f.read_to_string(&mut buf);
    if partial {
        if let Some(nl) = buf.find('\n') {
            return buf[nl + 1..].to_string();
        }
    }
    buf
}

pub(super) fn new_pane(session: &str) -> SessionPane {
    SessionPane {
        session: session.to_string(),
        // Short raw-session-id prefix for correlation.
        short: if session == "unscoped" {
            "unscoped".to_string()
        } else {
            session.chars().take(8).collect()
        },
        ..SessionPane::default()
    }
}

pub(super) fn fill_pane_from_hook(pane: &mut SessionPane, host: &str, stdin_json: &Value) {
    if pane.host.is_empty() {
        pane.host = host.to_string();
    }
    if pane.root.is_empty() {
        pane.root = stdin_json["cwd"]
            .as_str()
            .map(|cwd| crate::workspace::resolve(std::path::Path::new(cwd)).unwrap_or_default())
            .unwrap_or_default();
    }
}

pub(super) fn hook_session(v: &Value) -> Option<String> {
    [
        "session_id",
        "sessionId",
        "conversation_id",
        "conversationId",
        "thread_id",
        "threadId",
    ]
    .iter()
    .find_map(|key| v[*key].as_str())
    .filter(|s| !s.is_empty())
    .map(str::to_string)
}

pub(super) fn command_session(v: &Value) -> Option<String> {
    v["command"]["explicit_session"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn command_root(v: &Value) -> String {
    v["process"]["cwd"]
        .as_str()
        .map(|cwd| crate::workspace::resolve(std::path::Path::new(cwd)).unwrap_or_default())
        .unwrap_or_default()
}

pub(super) fn infer_command_session(
    panes: &BTreeMap<String, SessionPane>,
    agent: &str,
    root: &str,
) -> Option<String> {
    if agent.is_empty() || root.is_empty() {
        return None;
    }
    let matches = panes
        .values()
        .filter(|p| p.agent == agent && p.root == root && !p.session.is_empty())
        .map(|p| p.session.clone())
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

pub(super) fn ts_ms(v: &Value) -> u128 {
    v["timestamp"]["unix_ms"]
        .as_u64()
        .map(|n| n as u128)
        .unwrap_or(0)
}

pub(super) fn latest_ts(pane: &SessionPane) -> u128 {
    pane.lines.iter().map(|l| l.ts_ms).max().unwrap_or(0)
}

pub(super) fn fmt_rel_ts(ts_ms: u128, base_ms: u128) -> String {
    if ts_ms == 0 || base_ms == 0 {
        return "     ".to_string();
    }
    let delta = ts_ms.saturating_sub(base_ms) as f64 / 1000.0;
    let s = if delta < 10.0 {
        format!("+{:.1}s", delta)
    } else {
        format!("+{:.0}s", delta)
    };
    format!("{:>6}", s)
}

pub(super) fn truncate_str(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let mut out = String::new();
    for _ in 0..max {
        match chars.next() {
            Some(c) => out.push(c),
            None => return out,
        }
    }
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

pub(super) fn fixed_label(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count > width {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        format!("{:<width$}", s)
    }
}

pub(super) fn classify_hook(hook_type: &str, stdin: &Value) -> (String, String, String) {
    let short = hook_type.rsplit('/').next().unwrap_or(hook_type);

    let summary = match short {
        "user-prompt-submit" => stdin["prompt"]
            .as_str()
            .map(|p| truncate_str(p, 200))
            .unwrap_or_default(),
        "pre-tool-use" => {
            let tool = stdin["tool_name"].as_str().unwrap_or("?");
            if tool == "Bash" {
                let cmd = stdin["tool_input"]["command"].as_str().unwrap_or("");
                format!("Bash: {}", truncate_str(cmd, 120))
            } else {
                tool.to_string()
            }
        }
        "post-tool-use" => {
            let tool = stdin["tool_name"].as_str().unwrap_or("?");
            let response = stdin["tool_response"]
                .as_str()
                .map(|r| truncate_str(r.trim(), 100))
                .unwrap_or_default();
            if response.is_empty() {
                tool.to_string()
            } else {
                format!("{tool}: {response}")
            }
        }
        "stop" | "subagent-stop" => stdin["stop_reason"].as_str().unwrap_or("stop").to_string(),
        _ => stdin["transcript_path"]
            .as_str()
            .and_then(|p| std::path::Path::new(p).file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string(),
    };

    // Detail: human-readable expanded view
    let mut detail = String::new();
    if let Some(p) = stdin["prompt"].as_str() {
        detail.push_str(p);
    } else if let Some(tool) = stdin["tool_name"].as_str() {
        detail.push_str(&format!("tool: {tool}\n"));
        if let Some(input) = stdin.get("tool_input") {
            detail.push_str(&format!(
                "input:\n{}",
                serde_json::to_string_pretty(input).unwrap_or_default()
            ));
        }
        if let Some(resp) = stdin["tool_response"].as_str() {
            detail.push_str(&format!("\nresponse:\n{resp}"));
        }
    } else {
        detail = serde_json::to_string_pretty(stdin).unwrap_or_default();
    }

    (short.to_string(), summary, detail)
}

pub(super) fn kind_color(kind: DebugKind) -> ratatui::style::Color {
    use ratatui::style::Color;
    match kind {
        DebugKind::Hook => Color::Blue,
        DebugKind::Inject => Color::Green,
        DebugKind::Command => Color::Cyan,
        DebugKind::Error => Color::Red,
        DebugKind::Session => Color::DarkGray,
    }
}
