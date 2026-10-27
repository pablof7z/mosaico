//! Shared text renderers for channel output — used by both the CLI and the
//! Pi tool surface.  The Pi tool surface is the policy owner (see the module
//! doc in `daemon/server/pi_tools.rs`), so renderers live here rather than
//! inside `cli/`.

use anyhow::{Context, Result};
use owo_colors::OwoColorize as _;
use serde::Deserialize;
use serde_json::Value;
use std::fmt::Write as _;

// ── Notice rendering ─────────────────────────────────────────────────────────

/// Controls the follow-up hint emitted for an `untagged_agent_prefix` notice.
pub(crate) enum NoticeHint {
    /// The caller is a CLI user — suggest a shell command.
    Cli,
    /// The caller is an agent using the Pi tool surface — suggest the tool.
    Tool,
}

/// Extract the `notices` array from a send/reply result and return one line
/// per notice summary, plus a context-appropriate follow-up hint for
/// `untagged_agent_prefix` notices.
pub(crate) fn send_notice_lines(result: &Value, hint: NoticeHint) -> Result<Vec<String>> {
    let Some(notices) = result.get("notices").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    for notice in notices {
        let summary = notice
            .get("summary")
            .and_then(Value::as_str)
            .context("daemon returned notice without a summary")?;
        lines.push(summary.to_string());
        if notice.get("code").and_then(Value::as_str) != Some("untagged_agent_prefix") {
            continue;
        }
        let Some(agent) = notice.get("matched_agent").and_then(Value::as_str) else {
            continue;
        };
        let id = result
            .get("id")
            .and_then(Value::as_str)
            .context("daemon response missing id for notice command")?;
        match hint {
            NoticeHint::Cli => {
                let channel = notice
                    .get("channel")
                    .and_then(Value::as_str)
                    .context("daemon returned untagged_agent_prefix notice without channel")?;
                let message = format!("That message, {id}, was for you; I forgot to tag you.");
                lines.push(format!(
                    "To tag that agent now, run: `mosaico channel send --channel {} --tag {} --message {}`",
                    shell_quote(channel),
                    shell_quote(agent),
                    shell_quote(&message),
                ));
            }
            NoticeHint::Tool => {
                lines.push(format!(
                    "To tag that agent, call mosaico_send again with tags=[\"{agent}\"]"
                ));
            }
        }
    }
    Ok(lines)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

// ── Wait result rendering ─────────────────────────────────────────────────────

/// Render a `channel_wait` RPC result as the agent-native text string.
/// Returns the same `<mosaico>` envelope the CLI prints.
pub(crate) fn format_wait_result(result: &Value, now: u64) -> Result<String> {
    match result["outcome"].as_str() {
        Some("message") => Ok(render_wait_message(&result["message"], now)),
        Some("timeout") => {
            let seconds = result["timeout_secs"].as_u64().unwrap_or_default();
            let channels = result["channels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>();
            Ok(crate::injection::render_agent_wait_timeout(
                seconds, &channels,
            ))
        }
        other => anyhow::bail!("daemon returned invalid channel wait outcome {other:?}"),
    }
}

/// Render the `message` object from a `channel_wait` result into a
/// `<mosaico><channel ref="…"><message …/></channel></mosaico>` string.
pub(crate) fn render_wait_message(message: &Value, now: u64) -> String {
    let channel = message["channel"].as_str().unwrap_or_default();
    let from = message["from_ref"]
        .as_str()
        .or_else(|| message["from_slug"].as_str())
        .unwrap_or_default();
    let recipients = message["recipient_refs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let mut out = String::from("<mosaico>");
    let _ = write!(
        out,
        "\n  <channel ref=\"{}\">",
        crate::agent_xml::attr(channel)
    );
    crate::agent_xml::write_message(
        &mut out,
        4,
        &crate::agent_xml::MessageElement {
            event_id: message["id"].as_str().unwrap_or_default(),
            from,
            recipients: &recipients,
            attachment_dir: message["attachment_dir"].as_str().unwrap_or_default(),
            body: message["body"].as_str().unwrap_or_default(),
            created_at: message["created_at"].as_u64().unwrap_or_default(),
            now,
        },
    );
    out.push_str("\n  </channel>\n</mosaico>");
    out
}

// ── Channel read row rendering ────────────────────────────────────────────────

/// Render one streamed `channel_read` row into a single text line (plus an
/// optional attachments line).  Pass `use_color = false` for the Pi tool
/// surface; `true` uses ANSI colours keyed to the sender's pubkey.
pub(crate) fn render_channel_read_row(item: &Value, use_color: bool) -> String {
    let from_ref = item["from_ref"].as_str().unwrap_or_default();
    let slug = item["from_slug"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| from_ref.to_string());
    let host = item["host"].as_str().filter(|s| !s.is_empty());
    let sender = match host {
        Some(host) => format!("<{slug}@{host}>"),
        None => format!("<@{slug}>"),
    };
    let sender = color_by_pubkey(&sender, from_ref, use_color);
    let body = item["body"].as_str().unwrap_or_default().trim_end();
    let ts = item["created_at"].as_u64().unwrap_or(0);
    let mut text = format!("{sender} {body}");
    if item["truncated"].as_bool().unwrap_or(false) {
        if let Some(id) = item["id"].as_str().filter(|s| !s.is_empty()) {
            text.push_str(&format!(
                "\n[message truncated; run `mosaico channel read --id {id}`]"
            ));
        }
    }
    let mut rendered = format!("{text} [{}]", crate::util::format_local_datetime(ts));
    if let Some(directory) = item["attachment_dir"].as_str().filter(|d| !d.is_empty()) {
        rendered.push_str(&format!("\n[attachments: {directory}]"));
    }
    rendered
}

fn color_by_pubkey(text: &str, pubkey: &str, use_color: bool) -> String {
    if !use_color || pubkey.is_empty() {
        return text.to_string();
    }
    let hash = pubkey.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| {
        acc.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(b)
    });
    match hash % 6 {
        0 => text.cyan().to_string(),
        1 => text.green().to_string(),
        2 => text.yellow().to_string(),
        3 => text.magenta().to_string(),
        4 => text.blue().to_string(),
        _ => text.red().to_string(),
    }
}

// ── Channel search response rendering ────────────────────────────────────────

pub(crate) fn format_search_response(response: &Value) -> Result<String> {
    format_search_response_at(response, crate::util::now_secs())
}

pub(crate) fn format_search_response_at(response: &Value, now: u64) -> Result<String> {
    let response: SearchResponse = serde_json::from_value(response.clone())?;
    let mut out = String::from("<mosaico>");
    for channel in response.channels {
        let _ = write!(
            out,
            "\n  <channel ref=\"{}\">",
            crate::agent_xml::attr(&channel.channel_ref)
        );
        for message in channel.messages {
            crate::agent_xml::write_message(
                &mut out,
                4,
                &crate::agent_xml::MessageElement {
                    event_id: &message.id,
                    from: &message.from,
                    recipients: &message.recipients,
                    attachment_dir: &message.attachment_dir,
                    body: &message.body,
                    created_at: message.created_at,
                    now,
                },
            );
        }
        out.push_str("\n  </channel>");
    }
    if let Some(cursor) = response.next_cursor {
        let _ = write!(
            out,
            "\n  <next cursor=\"{}\" />",
            crate::agent_xml::attr(&cursor)
        );
    }
    out.push_str("\n</mosaico>");
    Ok(out)
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    channels: Vec<SearchChannel>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct SearchChannel {
    #[serde(rename = "ref")]
    channel_ref: String,
    #[serde(default)]
    messages: Vec<SearchMessage>,
}

#[derive(Deserialize)]
struct SearchMessage {
    id: String,
    from: String,
    #[serde(default)]
    recipients: Vec<String>,
    body: String,
    #[serde(default)]
    attachment_dir: String,
    created_at: u64,
}

// ── Channel list rendering ────────────────────────────────────────────────────

pub(crate) fn format_channel_list(projection: &Value) -> String {
    let mut output = String::new();
    let sections = projection["sections"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut printed = false;
    for section in sections {
        let channels = section["channels"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        if channels.is_empty() {
            continue;
        }
        if printed {
            output.push('\n');
        }
        let _ = writeln!(
            output,
            "{}",
            section["title"].as_str().unwrap_or("Channels")
        );
        for channel in channels {
            render_channel_entry(&mut output, channel, 0);
        }
        printed = true;
    }
    if !printed {
        output.push_str("No channels found.\n");
    }
    output
}

fn render_channel_entry(output: &mut String, channel: &Value, depth: usize) {
    let path = channel["path"].as_str().unwrap_or("");
    if path.is_empty() {
        return;
    }
    let suffix = channel["subchannels"]
        .as_u64()
        .map(|count| format!(" + {count} subchannel{}", if count == 1 { "" } else { "s" }))
        .unwrap_or_default();
    let mut details = Vec::new();
    if let Some(about) = channel["about"].as_str().filter(|a| !a.is_empty()) {
        details.push(about.to_string());
    }
    if let Some(agents) = channel["agents"].as_u64() {
        details.push(format!(
            "{agents} agent{}",
            if agents == 1 { "" } else { "s" }
        ));
    }
    if let Some(activity) = channel["last_activity"].as_str().filter(|a| !a.is_empty()) {
        details.push(format!("active {activity}"));
    }
    let details = if details.is_empty() {
        String::new()
    } else {
        format!("  — {}", details.join(" · "))
    };
    let _ = writeln!(output, "{}{path}{suffix}{details}", "  ".repeat(depth));
    for child in channel["children"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        render_channel_entry(output, child, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cli_hint_builds_shell_command() {
        let result = json!({
            "id": "abcdef",
            "notices": [{
                "level": "warn",
                "code": "untagged_agent_prefix",
                "summary": "WARN: published ambient chat",
                "matched_agent": "drift-codex",
                "channel": "#mosaico/reviews"
            }]
        });
        let lines = send_notice_lines(&result, NoticeHint::Cli).unwrap();
        assert_eq!(lines[0], "WARN: published ambient chat");
        assert!(lines[1].contains("mosaico channel send"), "{}", lines[1]);
        assert!(lines[1].contains("--tag 'drift-codex'"), "{}", lines[1]);
    }

    #[test]
    fn tool_hint_suggests_mosaico_send_with_tags() {
        let result = json!({
            "id": "abcdef",
            "notices": [{
                "level": "warn",
                "code": "untagged_agent_prefix",
                "summary": "WARN: published ambient chat",
                "matched_agent": "drift-codex",
                "channel": "#mosaico/reviews"
            }]
        });
        let lines = send_notice_lines(&result, NoticeHint::Tool).unwrap();
        assert_eq!(lines[0], "WARN: published ambient chat");
        assert!(lines[1].contains("mosaico_send"), "{}", lines[1]);
        assert!(lines[1].contains("tags="), "{}", lines[1]);
        assert!(!lines[1].contains("mosaico channel send"), "{}", lines[1]);
    }

    #[test]
    fn absent_notices_returns_empty() {
        let result = json!({"id": "abc"});
        assert!(send_notice_lines(&result, NoticeHint::Cli)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_response_groups_channels_and_uses_canonical_message_elements() {
        let response = json!({
            "channels": [{
                "ref": "#fava/research&design",
                "messages": [{
                    "id": "4e91c0b7f2de",
                    "from": "Pablo",
                    "recipients": ["reviewer"],
                    "body": "landed <the> commit",
                    "attachment_dir": "/tmp/mosaico-files/4e91c0",
                    "created_at": 9_940,
                }]
            }],
            "next_cursor": "page&2",
        });
        let xml = format_search_response_at(&response, 10_000).unwrap();
        assert!(xml.starts_with("<mosaico>\n  <channel ref=\"#fava/research&amp;design\">"));
        assert!(xml.contains(
            "<message from=\"@Pablo\" id=\"4e91c0\" for=\"@reviewer\" attachment-dir=\"/tmp/mosaico-files/4e91c0\" age=\"1m\">landed &lt;the&gt; commit</message>"
        ));
        assert!(xml.contains("<next cursor=\"page&amp;2\" />"));
        assert!(xml.ends_with("\n</mosaico>"));
    }

    #[test]
    fn channel_list_renders_sections_and_nesting() {
        let projection = json!({
            "sections": [{
                "kind": "own",
                "title": "Your workspace",
                "channels": [{
                    "path": "#root",
                    "about": "Root",
                    "agents": 2,
                    "last_activity": "3 min ago",
                    "children": [{"path": "#root/review", "about": "Reviews"}],
                }],
            }],
        });
        assert_eq!(
            format_channel_list(&projection),
            concat!(
                "Your workspace\n",
                "#root  — Root · 2 agents · active 3 min ago\n",
                "  #root/review  — Reviews\n",
            )
        );
    }
}
