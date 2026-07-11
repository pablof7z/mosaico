use anyhow::Result;
use owo_colors::OwoColorize as _;
use std::io::IsTerminal as _;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PtyListRow {
    display_id: String,
    agent: String,
    live: bool,
    command: Vec<String>,
}

pub(super) fn list() -> Result<()> {
    let rows = daemon_rows().unwrap_or_else(local_rows);
    print!("{}", render_rows_with_color(&rows, stdout_color_enabled()));
    Ok(())
}

fn stdout_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

fn daemon_rows() -> Option<Vec<PtyListRow>> {
    let status =
        crate::daemon::blocking::call_no_spawn("pty_status", serde_json::json!({})).ok()?;
    let sessions =
        crate::daemon::blocking::call_no_spawn("agents_list_sessions", serde_json::json!({}))
            .unwrap_or_else(|_| serde_json::json!({}));
    Some(rows_from_status(&status, &sessions))
}

fn local_rows() -> Vec<PtyListRow> {
    crate::pty::read_all_metadata()
        .into_iter()
        .map(|meta| PtyListRow {
            live: crate::pty::is_live(&meta.id),
            display_id: meta.id,
            agent: meta.agent,
            command: meta.command,
        })
        .collect()
}

fn rows_from_status(status: &serde_json::Value, sessions: &serde_json::Value) -> Vec<PtyListRow> {
    let session_names = session_display_names(sessions);
    status["endpoints"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|value| {
            let pty_id = value["pty_id"].as_str().filter(|s| !s.is_empty())?;
            let display_id = value["display_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    value["session_id"]
                        .as_str()
                        .and_then(|id| session_names.get(id).map(String::as_str))
                })
                .unwrap_or(pty_id);
            Some(PtyListRow {
                display_id: display_id.to_string(),
                agent: value["agent"].as_str().unwrap_or("?").to_string(),
                live: value["live"].as_bool().unwrap_or(false),
                command: value["command"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn session_display_names(
    sessions: &serde_json::Value,
) -> std::collections::HashMap<String, String> {
    sessions["sessions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|value| {
            let session_id = value["session_id"].as_str().filter(|s| !s.is_empty())?;
            let display = value["agent"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| value["handle"].as_str().filter(|s| !s.is_empty()))?;
            Some((session_id.to_string(), display.to_string()))
        })
        .collect()
}

#[cfg(test)]
fn render_rows(rows: &[PtyListRow]) -> String {
    render_rows_with_color(rows, false)
}

fn render_rows_with_color(rows: &[PtyListRow], color: bool) -> String {
    if rows.is_empty() {
        return "No portable-pty sessions found.\n".to_string();
    }
    let header_id = format!("{:<28} ", "id");
    let header_agent = format!("{:<10} ", "agent");
    let header_live = format!("{:<5} ", "live");
    let mut out = if color {
        format!(
            "{}{}{}{}\n",
            header_id.bold().cyan(),
            header_agent.bold().cyan(),
            header_live.bold().cyan(),
            "command".bold()
        )
    } else {
        format!("{header_id}{header_agent}{header_live}command\n")
    };
    for row in rows {
        let live = if row.live { "yes" } else { "no" };
        let id = format!("{:<28} ", row.display_id);
        let agent = format!("{:<10} ", row.agent);
        let live = format!("{live:<5} ");
        let command = row.command.join(" ");
        if color {
            let live = if row.live {
                live.bold().green().to_string()
            } else {
                live.dimmed().to_string()
            };
            out.push_str(&format!(
                "{}{}{}{}\n",
                id.cyan(),
                agent.magenta(),
                live,
                command.dimmed()
            ));
        } else {
            out.push_str(&format!("{id}{agent}{live}{command}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_prefers_kind0_display_id_over_raw_pty_id() {
        let status = serde_json::json!({
            "endpoints": [{
                "pty_id": "haiku-1783694933-98782",
                "display_id": "haiku/willow-echo-042",
                "agent": "haiku",
                "live": true,
                "command": ["haiku"]
            }]
        });

        let rows = rows_from_status(&status, &serde_json::json!({}));
        let rendered = render_rows(&rows);

        assert_eq!(rows[0].display_id, "haiku/willow-echo-042");
        assert!(rendered.contains("haiku/willow-echo-042"));
        assert!(!rendered.contains("haiku-1783694933-98782"));
    }

    #[test]
    fn render_falls_back_to_raw_pty_id_when_daemon_has_no_session_name() {
        let status = serde_json::json!({
            "endpoints": [{
                "pty_id": "haiku-1783694933-98782",
                "agent": "haiku",
                "live": false,
                "command": ["haiku"]
            }]
        });

        let rows = rows_from_status(&status, &serde_json::json!({}));
        let rendered = render_rows(&rows);

        assert_eq!(rows[0].display_id, "haiku-1783694933-98782");
        assert!(rendered.contains("haiku-1783694933-98782"));
    }

    #[test]
    fn render_uses_session_name_from_old_daemon_payloads() {
        let status = serde_json::json!({
            "endpoints": [{
                "pty_id": "haiku-1783694933-98782",
                "session_id": "sess-1",
                "agent": "haiku",
                "live": true,
                "command": ["haiku"]
            }]
        });
        let sessions = serde_json::json!({
            "sessions": [{
                "session_id": "sess-1",
                "agent": "haiku/willow-echo-042"
            }]
        });

        let rows = rows_from_status(&status, &sessions);
        let rendered = render_rows(&rows);

        assert_eq!(rows[0].display_id, "haiku/willow-echo-042");
        assert!(rendered.contains("haiku/willow-echo-042"));
        assert!(!rendered.contains("haiku-1783694933-98782"));
    }

    #[test]
    fn render_colorizes_columns_without_changing_plain_output() {
        let rows = vec![PtyListRow {
            display_id: "haiku-opal-spark-938".to_string(),
            agent: "haiku".to_string(),
            live: true,
            command: vec![
                "claude".to_string(),
                "--model".to_string(),
                "haiku".to_string(),
            ],
        }];

        let plain = render_rows(&rows);
        let colored = render_rows_with_color(&rows, true);

        assert!(!plain.contains("\u{1b}["), "plain output: {plain:?}");
        assert!(colored.contains("\u{1b}["), "colored output: {colored:?}");
        assert_eq!(strip_ansi(&colored), plain);
    }

    fn strip_ansi(input: &str) -> String {
        let mut out = String::new();
        let mut chars = input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }
}
