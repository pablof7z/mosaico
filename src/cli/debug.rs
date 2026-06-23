use super::*;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Terminal,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(super) struct HookTailOpts {
    pub(super) projects: Vec<String>,
    pub(super) session: Option<String>,
    pub(super) panes: usize,
    pub(super) refresh: Duration,
}

#[derive(Clone, Debug)]
struct DebugLine {
    ts_ms: u128,
    kind: DebugKind,
    label: String,   // event type, e.g. "user-prompt-submit", "inject", "inbox send"
    summary: String, // smart one-liner shown in the timeline
    detail: String,  // full content for the detail panel (real newlines)
    ok: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DebugKind {
    Hook,
    Inject,
    Command,
    Error,
    Session,
}

#[derive(Clone, Debug, Default)]
struct SessionPane {
    session: String,
    short: String,
    project: String,
    agent: String,
    host: String,
    lines: Vec<DebugLine>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HookTailSnapshot {
    panes: Vec<SessionPane>,
    unscoped: Vec<DebugLine>,
    projects: Vec<String>,
    sessions: Vec<String>,
}

struct ProjectPopup {
    cursor: usize,
}

// line_cursor == usize::MAX means "follow tail" (auto-scroll to last line)
struct HookTailState {
    project_filters: BTreeSet<String>,
    session_filter: Option<String>,
    pane_limit: usize,
    focused: usize,
    focused_session: Option<String>, // session ID of the focused pane; stable across snapshot re-sorts
    focus_mode: bool,
    line_cursor: usize,
    detail_open: bool, // full-screen detail overlay for the selected line
    status: String,
    popup: Option<ProjectPopup>,
}

struct TuiTerminal;

impl TuiTerminal {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

pub(super) fn hook_tail(opts: HookTailOpts) -> Result<()> {
    let mut state = HookTailState {
        project_filters: opts.projects.into_iter().collect(),
        session_filter: opts.session,
        pane_limit: opts.panes.clamp(1, 24),
        focused: 0,
        focused_session: None,
        focus_mode: false,
        line_cursor: usize::MAX,
        detail_open: false,
        status: String::new(),
        popup: None,
    };

    let refresh = opts.refresh.max(Duration::from_millis(100));
    let snapshot = load_hook_tail_snapshot(&state.project_filters, &state.session_filter);

    let _terminal = TuiTerminal::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut next_refresh = Instant::now();
    let (snap_tx, snap_rx) = std::sync::mpsc::channel::<HookTailSnapshot>();
    let mut loading = false;
    let mut snapshot = snapshot;

    loop {
        while let Ok(new) = snap_rx.try_recv() {
            snapshot = new;
            loading = false;
        }

        // Re-anchor focused index to the same session after a snapshot re-sort.
        if let Some(ref sess) = state.focused_session {
            if let Some(idx) = snapshot.panes.iter().position(|p| &p.session == sess) {
                state.focused = idx;
            }
        }
        if state.focused >= snapshot.panes.len().max(1) {
            state.focused = snapshot.panes.len().saturating_sub(1);
            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
        }

        terminal.draw(|f| render_hook_tail(f, &snapshot, &state))?;

        let wait = next_refresh
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));

        if event::poll(wait)? {
            if let TermEvent::Key(key) = event::read()? {
                if state.detail_open {
                    // Any key closes the detail overlay
                    match key.code {
                        KeyCode::Char('q') => break,
                        _ => state.detail_open = false,
                    }
                } else if let Some(popup) = &mut state.popup {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('p') | KeyCode::Enter => {
                            state.popup = None;
                            next_refresh = Instant::now();
                        }
                        KeyCode::Up => {
                            if popup.cursor > 0 {
                                popup.cursor -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if popup.cursor + 1 < snapshot.projects.len() {
                                popup.cursor += 1;
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(project) = snapshot.projects.get(popup.cursor) {
                                if state.project_filters.contains(project) {
                                    state.project_filters.remove(project);
                                } else {
                                    state.project_filters.insert(project.clone());
                                }
                            }
                        }
                        KeyCode::Char('a') => {
                            state.project_filters.clear();
                        }
                        _ => {}
                    }
                } else if state.focus_mode {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Esc | KeyCode::Char('f') => {
                            state.focus_mode = false;
                        }
                        KeyCode::Enter => {
                            state.detail_open = true;
                        }
                        KeyCode::Up => {
                            let pane_len = snapshot
                                .panes
                                .get(state.focused)
                                .map(|p| p.lines.len())
                                .unwrap_or(0);
                            if state.line_cursor == usize::MAX {
                                state.line_cursor = pane_len.saturating_sub(2);
                            } else {
                                state.line_cursor = state.line_cursor.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            let pane_len = snapshot
                                .panes
                                .get(state.focused)
                                .map(|p| p.lines.len())
                                .unwrap_or(0);
                            let last = pane_len.saturating_sub(1);
                            if state.line_cursor >= last {
                                state.line_cursor = usize::MAX; // snap to tail
                            } else {
                                state.line_cursor += 1;
                            }
                        }
                        KeyCode::Tab | KeyCode::Right => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = (state.focused + 1) % n;
                            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
                            state.line_cursor = usize::MAX;
                        }
                        KeyCode::BackTab | KeyCode::Left => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = if state.focused == 0 {
                                n - 1
                            } else {
                                state.focused - 1
                            };
                            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
                            state.line_cursor = usize::MAX;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            state.pane_limit = (state.pane_limit + 1).min(24);
                        }
                        KeyCode::Char('-') => {
                            state.pane_limit = state.pane_limit.saturating_sub(1).max(1);
                            state.focused = state.focused.min(state.pane_limit.saturating_sub(1));
                            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
                        }
                        KeyCode::Tab | KeyCode::Right => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = (state.focused + 1) % n;
                            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
                        }
                        KeyCode::BackTab | KeyCode::Left => {
                            let n = snapshot.panes.len().min(state.pane_limit).max(1);
                            state.focused = if state.focused == 0 {
                                n - 1
                            } else {
                                state.focused - 1
                            };
                            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
                        }
                        KeyCode::Enter | KeyCode::Char('f') => {
                            state.focused_session = snapshot.panes.get(state.focused).map(|p| p.session.clone());
                            state.focus_mode = true;
                            state.line_cursor = usize::MAX;
                        }
                        KeyCode::Char('a') => {
                            state.project_filters.clear();
                            state.session_filter = None;
                            state.status = "filters cleared".to_string();
                            next_refresh = Instant::now();
                        }
                        KeyCode::Char('p') => {
                            state.popup = Some(ProjectPopup { cursor: 0 });
                        }
                        KeyCode::Char('s') => {
                            state.session_filter =
                                cycle_filter(state.session_filter.as_deref(), &snapshot.sessions);
                            state.status = match &state.session_filter {
                                Some(s) => format!("session filter: {s}"),
                                None => "session filter cleared".to_string(),
                            };
                            next_refresh = Instant::now();
                        }
                        _ => {}
                    }
                }
            }
        }

        if !loading && Instant::now() >= next_refresh {
            next_refresh = Instant::now() + refresh;
            loading = true;
            let tx = snap_tx.clone();
            let filter_p = state.project_filters.clone();
            let filter_s = state.session_filter.clone();
            std::thread::spawn(move || {
                let snap = load_hook_tail_snapshot(&filter_p, &filter_s);
                let _ = tx.send(snap);
            });
        }
    }

    Ok(())
}

fn cycle_filter(current: Option<&str>, values: &[String]) -> Option<String> {
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

pub(crate) fn load_hook_tail_snapshot(
    project_filters: &BTreeSet<String>,
    session_filter: &Option<String>,
) -> HookTailSnapshot {
    let mut panes: BTreeMap<String, SessionPane> = BTreeMap::new();
    seed_live_sessions(&mut panes);

    let home = crate::config::edge_home();
    let sessions_dir = home.join("sessions");
    let mut unscoped = Vec::new();
    if sessions_dir.is_dir() {
        // New layout: one directory per session under sessions/<id>/
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let dir_name = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let hint = if dir_name == "_unscoped" {
                    None
                } else {
                    Some(dir_name.as_str())
                };
                read_session_hook_log(&dir.join("hook-calls.jsonl"), &mut panes, hint);
                let cmd_unscoped =
                    read_session_command_log(&dir.join("command-calls.jsonl"), &mut panes, hint);
                unscoped.extend(cmd_unscoped);
            }
        }
    } else {
        // Legacy fallback: old monolithic files.
        read_hook_log(&home.join("hook-calls.jsonl"), &mut panes, 20_000_000);
        unscoped = read_command_log(&home.join("command-calls.jsonl"), &mut panes);
    }

    let mut projects = BTreeSet::new();
    let mut sessions = BTreeSet::new();
    for pane in panes.values() {
        if !pane.project.is_empty() {
            projects.insert(pane.project.clone());
        }
        if !pane.session.is_empty() {
            sessions.insert(pane.short.clone());
        }
    }

    let mut panes: Vec<SessionPane> = panes
        .into_values()
        .filter(|p| project_filters.is_empty() || project_filters.contains(&p.project))
        .filter(|p| match session_filter {
            Some(filter) => p.session == *filter || p.short == *filter,
            None => true,
        })
        .collect();
    for pane in &mut panes {
        pane.lines.sort_by_key(|l| l.ts_ms);
        if pane.lines.is_empty() {
            pane.lines.push(DebugLine {
                ts_ms: 0,
                kind: DebugKind::Session,
                label: "session".to_string(),
                summary: "no hook or command telemetry yet".to_string(),
                detail: String::new(),
                ok: None,
            });
        }
    }
    panes.sort_by(|a, b| latest_ts(b).cmp(&latest_ts(a)).then(a.short.cmp(&b.short)));

    HookTailSnapshot {
        panes,
        unscoped,
        projects: projects.into_iter().collect(),
        sessions: sessions.into_iter().collect(),
    }
}

fn seed_live_sessions(panes: &mut BTreeMap<String, SessionPane>) {
    let Ok(v) = crate::daemon::blocking::call_no_spawn(
        "who",
        serde_json::json!({
            "project": null,
            "all": false,
            "all_projects": true,
            "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        }),
    ) else {
        return;
    };
    for row in v["rows"].as_array().cloned().unwrap_or_default() {
        let session = row["session_id"].as_str().unwrap_or("").to_string();
        if session.is_empty() {
            continue;
        }
        let pane = panes
            .entry(session.clone())
            .or_insert_with(|| new_pane(&session));
        pane.project = row["project"].as_str().unwrap_or("").to_string();
        pane.agent = row["slug"].as_str().unwrap_or("").to_string();
        pane.host = row["host"].as_str().unwrap_or("").to_string();
    }
}

fn tail_read(path: &std::path::Path, max_bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let partial = len > max_bytes;
    if partial {
        if f.seek(SeekFrom::Start(len - max_bytes)).is_err() {
            return String::new();
        }
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

/// Read a per-session hook log (whole file, no tail limit).
/// `session_hint` is the session_id inferred from the directory name; used as a fallback
/// when stdin doesn't carry a session_id (rare, but possible for early events).
fn read_session_hook_log(
    path: &std::path::Path,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return;
    };
    parse_hook_log(&raw, panes, session_hint);
}

/// Legacy: read the global hook log with a byte-limit tail.
fn read_hook_log(
    path: &std::path::Path,
    panes: &mut BTreeMap<String, SessionPane>,
    max_bytes: u64,
) {
    let raw = tail_read(path, max_bytes);
    if raw.is_empty() {
        return;
    }
    parse_hook_log(&raw, panes, None);
}

fn parse_hook_log(
    raw: &str,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) {
    let mut hook_sessions: HashMap<String, String> = HashMap::new();
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["schema"].as_str() != Some("tenex-edge.hook-call.v1") {
            continue;
        }
        let call_id = v["call_id"].as_str().unwrap_or("").to_string();
        let ts_ms = ts_ms(&v);
        match v["phase"].as_str().unwrap_or("") {
            "received" => {
                let host = v["hook"]["host"].as_str().unwrap_or("");
                let hook_type = v["hook"]["type"].as_str().unwrap_or("");
                let stdin_json = &v["stdin"]["json"];
                let session = hook_session(stdin_json)
                    .or_else(|| session_hint.map(str::to_string))
                    .unwrap_or_else(|| "unscoped".to_string());
                hook_sessions.insert(call_id, session.clone());
                let pane = panes
                    .entry(session.clone())
                    .or_insert_with(|| new_pane(&session));
                fill_pane_from_hook(pane, host, stdin_json);
                let (label, summary, detail) = classify_hook(hook_type, stdin_json);
                pane.lines.push(DebugLine {
                    ts_ms,
                    kind: DebugKind::Hook,
                    label,
                    summary,
                    detail,
                    ok: None,
                });
            }
            "note" => {
                let note = v["note"].as_str().unwrap_or("note");
                let detail_val = &v["detail"];
                let session = detail_val["session"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| hook_sessions.get(&call_id).cloned())
                    .or_else(|| session_hint.map(str::to_string))
                    .unwrap_or_else(|| "unscoped".to_string());
                let pane = panes
                    .entry(session.clone())
                    .or_insert_with(|| new_pane(&session));
                if note == "context-injection" {
                    let full_text = detail_val["text"].as_str().unwrap_or("").to_string();
                    let summary = full_text
                        .lines()
                        .next()
                        .map(|l| truncate_str(l, 160))
                        .unwrap_or_default();
                    pane.lines.push(DebugLine {
                        ts_ms,
                        kind: DebugKind::Inject,
                        label: "inject".to_string(),
                        summary,
                        detail: full_text,
                        ok: None,
                    });
                } else {
                    pane.lines.push(DebugLine {
                        ts_ms,
                        kind: DebugKind::Hook,
                        label: note.to_string(),
                        summary: truncate_str(&detail_val.to_string(), 160),
                        detail: detail_val.to_string(),
                        ok: None,
                    });
                }
            }
            "finished" => {
                let ok = v["result"]["ok"].as_bool();
                // Skip successful completions — they're pure noise.
                if ok != Some(false) {
                    continue;
                }
                let session = hook_sessions
                    .get(&call_id)
                    .cloned()
                    .or_else(|| session_hint.map(str::to_string))
                    .unwrap_or_else(|| "unscoped".to_string());
                let pane = panes
                    .entry(session.clone())
                    .or_insert_with(|| new_pane(&session));
                let error = v["result"]["error"].as_str().unwrap_or("unknown error");
                pane.lines.push(DebugLine {
                    ts_ms,
                    kind: DebugKind::Error,
                    label: "error".to_string(),
                    summary: error.to_string(),
                    detail: error.to_string(),
                    ok,
                });
            }
            _ => {}
        }
    }
}

/// Read a per-session command log. `_unscoped` accumulates across all sessions
/// and can grow to gigabytes; tail-limit it the same way the legacy global file is.
fn read_session_command_log(
    path: &std::path::Path,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) -> Vec<DebugLine> {
    let raw = tail_read(path, 2_000_000);
    if raw.is_empty() {
        return Vec::new();
    }
    parse_command_log(&raw, panes, session_hint)
}

/// Legacy: read the global command log with a byte-limit tail.
fn read_command_log(
    path: &std::path::Path,
    panes: &mut BTreeMap<String, SessionPane>,
) -> Vec<DebugLine> {
    let raw = tail_read(path, 2_000_000);
    if raw.is_empty() {
        return Vec::new();
    }
    parse_command_log(&raw, panes, None)
}

fn parse_command_log(
    raw: &str,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) -> Vec<DebugLine> {
    let mut received: HashMap<String, Value> = HashMap::new();
    let mut unscoped = Vec::new();
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["schema"].as_str() != Some("tenex-edge.command-call.v1") {
            continue;
        }
        let call_id = v["call_id"].as_str().unwrap_or("").to_string();
        match v["phase"].as_str().unwrap_or("") {
            "received" => {
                received.insert(call_id, v);
            }
            "finished" => {
                let Some(start) = received.get(&call_id) else {
                    continue;
                };
                let project = command_project(start);
                let agent = start["env"]["TENEX_EDGE_AGENT"]
                    .as_str()
                    .or_else(|| start["env"]["TENEX_EDGE_AGENT_FALLBACK"].as_str())
                    .unwrap_or("")
                    .to_string();
                let session = command_session(start)
                    .or_else(|| infer_command_session(panes, &agent, &project))
                    .or_else(|| session_hint.map(str::to_string));
                let argv: Vec<&str> = start["command"]["argv"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                // Strip the binary name, show the subcommand
                let subcmd = argv.get(1..).unwrap_or(&[]).join(" ");
                let ok = v["result"]["ok"].as_bool();
                let summary = if ok == Some(false) {
                    let err = v["result"]["error"].as_str().unwrap_or("error");
                    format!("{subcmd}  ✗ {}", truncate_str(err, 80))
                } else {
                    subcmd.clone()
                };
                let detail = if let Some(err) = v["result"]["error"].as_str() {
                    format!("{}\n\nerror: {err}", argv.join(" "))
                } else {
                    argv.join(" ")
                };
                let entry = DebugLine {
                    ts_ms: ts_ms(&v),
                    kind: if ok == Some(false) {
                        DebugKind::Error
                    } else {
                        DebugKind::Command
                    },
                    label: "cmd".to_string(),
                    summary,
                    detail,
                    ok,
                };
                if let Some(session) = session {
                    let pane = panes
                        .entry(session.clone())
                        .or_insert_with(|| new_pane(&session));
                    if !project.is_empty() {
                        pane.project = project;
                    }
                    if !agent.is_empty() {
                        pane.agent = agent;
                    }
                    pane.lines.push(entry);
                } else {
                    unscoped.push(entry);
                }
            }
            _ => {}
        }
    }
    unscoped
}

fn classify_hook(hook_type: &str, stdin: &Value) -> (String, String, String) {
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

fn new_pane(session: &str) -> SessionPane {
    SessionPane {
        session: session.to_string(),
        short: if session == "unscoped" {
            "unscoped".to_string()
        } else {
            SessionId::from(session).to_string()
        },
        ..SessionPane::default()
    }
}

fn fill_pane_from_hook(pane: &mut SessionPane, host: &str, stdin_json: &Value) {
    if pane.host.is_empty() {
        pane.host = host.to_string();
    }
    if pane.project.is_empty() {
        pane.project = stdin_json["cwd"]
            .as_str()
            .map(|cwd| crate::project::resolve(std::path::Path::new(cwd)).unwrap_or_default())
            .unwrap_or_default();
    }
}

fn hook_session(v: &Value) -> Option<String> {
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

fn command_session(v: &Value) -> Option<String> {
    v["env"]["TENEX_EDGE_SESSION"]
        .as_str()
        .or_else(|| v["command"]["explicit_session"].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn command_project(v: &Value) -> String {
    v["process"]["cwd"]
        .as_str()
        .map(|cwd| crate::project::resolve(std::path::Path::new(cwd)).unwrap_or_default())
        .unwrap_or_default()
}

fn infer_command_session(
    panes: &BTreeMap<String, SessionPane>,
    agent: &str,
    project: &str,
) -> Option<String> {
    if agent.is_empty() || project.is_empty() {
        return None;
    }
    let matches = panes
        .values()
        .filter(|p| p.agent == agent && p.project == project && !p.session.is_empty())
        .map(|p| p.session.clone())
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

fn ts_ms(v: &Value) -> u128 {
    v["timestamp"]["unix_ms"]
        .as_u64()
        .map(|n| n as u128)
        .unwrap_or(0)
}

fn latest_ts(pane: &SessionPane) -> u128 {
    pane.lines.iter().map(|l| l.ts_ms).max().unwrap_or(0)
}

fn fmt_rel_ts(ts_ms: u128, base_ms: u128) -> String {
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

fn truncate_str(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let mut out = String::new();
    for _ in 0..max {
        match chars.next() {
            Some(c) => out.push(c),
            None => return out,
        }
    }
    if chars.next().is_some() {
        out.push_str("…");
    }
    out
}

fn fixed_label(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count > width {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        format!("{:<width$}", s)
    }
}

// ─── rendering ───────────────────────────────────────────────────────────────

fn render_hook_tail(f: &mut ratatui::Frame, snapshot: &HookTailSnapshot, state: &HookTailState) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let project_label: String = if state.project_filters.is_empty() {
        "*".to_string()
    } else {
        state
            .project_filters
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    };
    let session = state.session_filter.as_deref().unwrap_or("*");
    let title = Line::from(vec![
        Span::styled(
            "tenex-edge debug hook-tail",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  project="),
        Span::styled(project_label, Style::default().fg(Color::Yellow)),
        Span::raw("  session="),
        Span::styled(session, Style::default().fg(Color::Yellow)),
        Span::raw("  panes="),
        Span::styled(
            state.pane_limit.to_string(),
            Style::default().fg(Color::Yellow),
        ),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    if state.focus_mode {
        render_focus(f, chunks[1], snapshot, state);
    } else {
        render_grid(f, chunks[1], snapshot, state);
    }

    let hints = if state.detail_open {
        "[any key] close"
    } else if state.popup.is_some() {
        "[↑↓] move  [space] toggle  [a] clear  [esc/p] close"
    } else if state.focus_mode {
        "[↑↓] select  [enter] open  [tab/←/→] pane  [f/esc] exit zoom  [q] quit"
    } else {
        "[enter/f] zoom  [tab/←/→] pane  [+/-] panes  [p] projects  [s] session  [a] clear  [q] quit"
    };
    let status = if state.status.is_empty()
        || state.popup.is_some()
        || state.focus_mode
        || state.detail_open
    {
        hints.to_string()
    } else {
        format!("{}  {}", state.status, hints)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            status,
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[2],
    );

    if state.popup.is_some() {
        render_project_popup(f, area, snapshot, state);
    }
    if state.detail_open {
        render_detail_overlay(f, area, snapshot, state);
    }
}

fn centered_rect(percent_x: u16, max_height: u16, r: Rect) -> Rect {
    let height = max_height.min(r.height.saturating_sub(4));
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((r.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1]);
    horiz[1]
}

fn render_detail_overlay(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let pane = snapshot.panes.get(state.focused);
    let n = pane.map(|p| p.lines.len()).unwrap_or(0);
    let selected = if state.line_cursor == usize::MAX || state.line_cursor >= n {
        n.saturating_sub(1)
    } else {
        state.line_cursor
    };
    let line = pane.and_then(|p| p.lines.get(selected));
    let (label, detail) = match line {
        Some(l) => (l.label.as_str(), l.detail.as_str()),
        None => ("detail", ""),
    };

    let overlay = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(3),
    };
    f.render_widget(Clear, overlay);

    let text_lines: Vec<Line> = detail
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::White),
            ))
        })
        .collect();

    f.render_widget(
        Paragraph::new(text_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(format!(" {label}  [any key] close ")),
            )
            .wrap(Wrap { trim: false }),
        overlay,
    );
}

fn render_project_popup(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let Some(popup) = &state.popup else { return };
    let popup_area = centered_rect(50, (snapshot.projects.len() as u16 + 4).max(6), area);
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = snapshot
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let checked = state.project_filters.contains(p);
            let focused = i == popup.cursor;
            let prefix = if checked { " [x] " } else { " [ ] " };
            let style = if focused {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if checked {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!("{}{}", prefix, p)).style(style)
        })
        .collect();

    let title = if state.project_filters.is_empty() {
        " Projects (all) ".to_string()
    } else {
        format!(" Projects ({} selected) ", state.project_filters.len())
    };

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(title),
        ),
        popup_area,
    );
}

fn render_grid(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let count = snapshot.panes.len().min(state.pane_limit);
    if count == 0 {
        render_empty(f, area, snapshot);
        return;
    }
    let rects = grid_rects(area, count);
    for (i, rect) in rects.into_iter().enumerate() {
        if let Some(pane) = snapshot.panes.get(i) {
            render_pane_grid(f, rect, pane, i == state.focused);
        }
    }
}

fn render_focus(
    f: &mut ratatui::Frame,
    area: Rect,
    snapshot: &HookTailSnapshot,
    state: &HookTailState,
) {
    let Some(pane) = snapshot.panes.get(state.focused) else {
        render_empty(f, area, snapshot);
        return;
    };

    let n = pane.lines.len();
    let selected = if state.line_cursor == usize::MAX || state.line_cursor >= n {
        n.saturating_sub(1)
    } else {
        state.line_cursor
    };

    // Height of detail panel: clamp to [4, 12] based on content
    let selected_line = pane.lines.get(selected);
    let detail_line_count = selected_line
        .map(|l| l.detail.lines().count().max(1))
        .unwrap_or(1);
    let detail_height = (detail_line_count as u16 + 2).clamp(4, 12);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(detail_height)])
        .split(area);

    render_pane_focus(f, chunks[0], pane, selected);
    render_detail_panel(f, chunks[1], selected_line);
}

fn render_empty(f: &mut ratatui::Frame, area: Rect, snapshot: &HookTailSnapshot) {
    let mut lines = vec![Line::from("No session telemetry yet.")];
    if !snapshot.unscoped.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Recent unscoped commands:"));
        let base = snapshot.unscoped.first().map(|l| l.ts_ms).unwrap_or(0);
        for line in snapshot.unscoped.iter().rev().take(8) {
            lines.push(render_timeline_line(line, base, false));
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("debug")),
        area,
    );
}

fn pane_title(pane: &SessionPane) -> String {
    match (pane.agent.as_str(), pane.project.as_str()) {
        ("", "") => pane.short.clone(),
        (agent, "") => format!("{} [{}]", agent, pane.short),
        ("", project) => format!("{} [{}]", project, pane.short),
        (agent, project) => format!("{}@{} [{}]", agent, project, pane.short),
    }
}

fn render_pane_grid(f: &mut ratatui::Frame, area: Rect, pane: &SessionPane, focused: bool) {
    let border_color = if focused {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let inner_h = area.height.saturating_sub(2) as usize;
    let base_ts = pane.lines.first().map(|l| l.ts_ms).unwrap_or(0);
    let start = pane.lines.len().saturating_sub(inner_h);
    let lines: Vec<Line> = pane
        .lines
        .iter()
        .skip(start)
        .map(|l| render_timeline_line(l, base_ts, false))
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(pane_title(pane)),
        ),
        area,
    );
}

fn render_pane_focus(f: &mut ratatui::Frame, area: Rect, pane: &SessionPane, selected: usize) {
    let inner_h = area.height.saturating_sub(2) as usize;
    let n = pane.lines.len();
    // Scroll to keep selected visible
    let scroll = if selected < inner_h {
        0
    } else {
        selected - inner_h + 1
    };
    let base_ts = pane.lines.first().map(|l| l.ts_ms).unwrap_or(0);
    let lines: Vec<Line> = pane
        .lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(inner_h)
        .map(|(i, l)| render_timeline_line(l, base_ts, i == selected))
        .collect();
    let title = format!("{} ({}/{})", pane_title(pane), selected + 1, n);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(title),
        ),
        area,
    );
}

fn render_detail_panel(f: &mut ratatui::Frame, area: Rect, line: Option<&DebugLine>) {
    let (label, text, color) = match line {
        None => ("detail", String::new(), Color::DarkGray),
        Some(l) => {
            let c = kind_color(l.kind);
            (l.label.as_str(), l.detail.clone(), c)
        }
    };
    let text_lines: Vec<Line> = text
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::White),
            ))
        })
        .collect();
    f.render_widget(
        Paragraph::new(text_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(color))
                    .title(format!(" {label} ")),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_timeline_line(line: &DebugLine, base_ts: u128, selected: bool) -> Line<'static> {
    let ts = fmt_rel_ts(line.ts_ms, base_ts);
    let label = fixed_label(&line.label, 18);
    let color = kind_color(line.kind);
    let bg = if selected {
        Color::Rgb(40, 40, 60)
    } else {
        Color::Reset
    };

    // User prompt text gets a brighter color so it's easy to scan
    let summary_color = match line.kind {
        DebugKind::Hook if line.label == "user-prompt-submit" => Color::LightYellow,
        DebugKind::Hook
            if line.label.starts_with("pre-tool-use")
                || line.label.starts_with("post-tool-use") =>
        {
            Color::Gray
        }
        DebugKind::Inject => Color::Gray,
        DebugKind::Command => Color::LightCyan,
        DebugKind::Error => Color::LightRed,
        _ => Color::Gray,
    };

    let base = Style::default().bg(bg);
    Line::from(vec![
        Span::styled(ts, base.fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(label, base.fg(color).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(line.summary.clone(), base.fg(summary_color)),
    ])
}

fn kind_color(kind: DebugKind) -> Color {
    match kind {
        DebugKind::Hook => Color::Blue,
        DebugKind::Inject => Color::Green,
        DebugKind::Command => Color::Cyan,
        DebugKind::Error => Color::Red,
        DebugKind::Session => Color::DarkGray,
    }
}

fn grid_rects(area: Rect, count: usize) -> Vec<Rect> {
    let cols = (count as f64).sqrt().ceil() as usize;
    let cols = cols.max(1);
    let rows = count.div_ceil(cols).max(1);
    let row_constraints = even_constraints(rows);
    let col_constraints = even_constraints(cols);
    let row_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(area);
    let mut rects = Vec::new();
    for row in row_rects.iter() {
        let cols_rects = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints.clone())
            .split(*row);
        for col in cols_rects.iter() {
            if rects.len() < count {
                rects.push(*col);
            }
        }
    }
    rects
}

fn even_constraints(n: usize) -> Vec<Constraint> {
    (0..n)
        .map(|_| Constraint::Ratio(1, n as u32))
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_session_prefers_env_then_explicit_flag() {
        let v = serde_json::json!({
            "env": {"TENEX_EDGE_SESSION": "env-session"},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("env-session"));

        let v = serde_json::json!({
            "env": {},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("flag-session"));
    }

    #[test]
    fn hook_session_accepts_codex_field_variants() {
        let v = serde_json::json!({"conversation_id": "codex-session"});
        assert_eq!(hook_session(&v).as_deref(), Some("codex-session"));
    }

    #[test]
    fn command_session_can_infer_unique_live_agent_project() {
        let mut panes = BTreeMap::new();
        panes.insert(
            "session-a".to_string(),
            SessionPane {
                session: "session-a".to_string(),
                project: "proj".to_string(),
                agent: "coder".to_string(),
                ..SessionPane::default()
            },
        );
        assert_eq!(
            infer_command_session(&panes, "coder", "proj").as_deref(),
            Some("session-a")
        );

        panes.insert(
            "session-b".to_string(),
            SessionPane {
                session: "session-b".to_string(),
                project: "proj".to_string(),
                agent: "coder".to_string(),
                ..SessionPane::default()
            },
        );
        assert_eq!(infer_command_session(&panes, "coder", "proj"), None);
    }
}
