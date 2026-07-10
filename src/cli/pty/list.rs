use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PtyListRow {
    display_id: String,
    agent: String,
    live: bool,
    command: Vec<String>,
}

pub(super) fn list() -> Result<()> {
    let rows = daemon_rows().unwrap_or_else(local_rows);
    print!("{}", render_rows(&rows));
    Ok(())
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

fn render_rows(rows: &[PtyListRow]) -> String {
    if rows.is_empty() {
        return "No portable-pty sessions found.\n".to_string();
    }
    let mut out = format!("{:<28} {:<10} {:<5} command\n", "id", "agent", "live");
    for row in rows {
        let live = if row.live { "yes" } else { "no" };
        out.push_str(&format!(
            "{:<28} {:<10} {:<5} {}\n",
            row.display_id,
            row.agent,
            live,
            row.command.join(" ")
        ));
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
}
