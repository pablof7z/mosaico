use anyhow::Result;
use clap::Args;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt::Write as _;

#[derive(Args)]
pub(in crate::cli) struct ChannelSearchArgs {
    /// Match an author identity. Repeat to match any author.
    #[arg(long = "from", value_name = "IDENTITY")]
    pub(in crate::cli) from: Vec<String>,
    /// Match an explicit recipient identity. Repeat to match any recipient.
    #[arg(long = "to", value_name = "IDENTITY")]
    pub(in crate::cli) to: Vec<String>,
    /// Match a case-insensitive literal body substring. Repeat to match any text.
    #[arg(long, value_name = "TEXT")]
    pub(in crate::cli) contains: Vec<String>,
    /// Search this channel and its descendants. Repeat to search any subtree.
    /// Omit, or pass `#`, to search every channel in the local database. Quote
    /// paths in the shell: `'#nmp/research'`.
    #[arg(
        long,
        value_name = "CHANNEL",
        value_parser = super::admin::parse_channel_path
    )]
    pub(in crate::cli) channel: Vec<String>,
    /// Match messages at or after this Unix timestamp or relative duration.
    #[arg(long, value_name = "TIME", value_parser = parse_search_time)]
    pub(in crate::cli) since: Option<u64>,
    /// Match messages at or before this Unix timestamp or relative duration.
    #[arg(long, value_name = "TIME", value_parser = parse_search_time)]
    pub(in crate::cli) until: Option<u64>,
    /// Maximum messages to return.
    #[arg(long, value_name = "COUNT")]
    pub(in crate::cli) limit: Option<u64>,
    /// Continue an earlier search page using its opaque cursor.
    #[arg(
        long,
        value_name = "CURSOR",
        conflicts_with_all = ["from", "to", "contains", "channel", "since", "until", "limit"]
    )]
    pub(in crate::cli) cursor: Option<String>,
}

pub(in crate::cli) async fn channel_search(args: ChannelSearchArgs) -> Result<()> {
    let response = daemon_search(search_params(
        args.from,
        args.to,
        args.contains,
        args.channel,
        args.since,
        args.until,
        args.limit,
        args.cursor,
    ))
    .await?;
    println!("{}", render_response(&response)?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::cli) fn search_params(
    from: Vec<String>,
    to: Vec<String>,
    contains: Vec<String>,
    channels: Vec<String>,
    since: Option<u64>,
    until: Option<u64>,
    limit: Option<u64>,
    cursor: Option<String>,
) -> Value {
    json!({
        "from": from,
        "to": to,
        "contains": contains,
        "channels": channels,
        "since": since,
        "until": until,
        "limit": limit,
        "cursor": cursor,
    })
}

pub(in crate::cli) async fn daemon_search(params: Value) -> Result<Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call("channel_search", params).await
}

pub(in crate::cli) fn parse_search_time(raw: &str) -> Result<u64, String> {
    if let Ok(timestamp) = raw.parse::<u64>() {
        return Ok(timestamp);
    }
    let value = raw.trim();
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let amount = number
        .parse::<u64>()
        .map_err(|_| format!("invalid time {raw:?}; use a Unix timestamp or duration like 2h"))?;
    let factor = match unit {
        "s" | "S" => 1,
        "m" | "M" => 60,
        "h" | "H" => 60 * 60,
        "d" | "D" => 24 * 60 * 60,
        _ => {
            return Err(format!(
                "invalid time {raw:?}; duration units are s, m, h, or d"
            ))
        }
    };
    let seconds = amount
        .checked_mul(factor)
        .ok_or_else(|| format!("duration {raw:?} is too large"))?;
    Ok(crate::util::now_secs().saturating_sub(seconds))
}

pub(in crate::cli) fn render_response(response: &Value) -> Result<String> {
    render_response_at(response, crate::util::now_secs())
}

fn render_response_at(response: &Value, now: u64) -> Result<String> {
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
                    event_id: &message.event_id,
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
    event_id: String,
    from: String,
    #[serde(default)]
    recipients: Vec<String>,
    body: String,
    #[serde(default)]
    attachment_dir: String,
    created_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_time_accepts_unix_and_rejects_unknown_units() {
        assert_eq!(parse_search_time("1785348600").unwrap(), 1_785_348_600);
        assert!(parse_search_time("2w").unwrap_err().contains("units"));
        assert!(parse_search_time("soon")
            .unwrap_err()
            .contains("invalid time"));
    }

    #[test]
    fn search_params_are_query_only_without_caller_or_workspace_fields() {
        let params = search_params(
            vec!["@Pablo".into()],
            Vec::new(),
            vec!["commit".into()],
            vec!["#".into()],
            Some(10),
            Some(20),
            Some(5),
            None,
        );
        assert_eq!(params["from"], json!(["@Pablo"]));
        assert!(params.get("session").is_none());
        assert!(params.get("cwd").is_none());
        assert!(params.get("workspace").is_none());
    }

    #[test]
    fn response_groups_channels_and_uses_canonical_message_elements() {
        let response = json!({
            "channels": [{
                "ref": "#nmp/research&design",
                "messages": [{
                    "event_id": "4e91c0b7f2de",
                    "from": "Pablo",
                    "recipients": ["reviewer"],
                    "body": "landed <the> commit",
                    "attachment_dir": "/tmp/mosaico-files/4e91c0",
                    "created_at": 9_940,
                }]
            }, {
                "ref": "#nmp/archive",
                "messages": [{
                    "event_id": "7bc421123456",
                    "from": "reviewer",
                    "recipients": [],
                    "body": "approved",
                    "created_at": 6_000,
                }]
            }],
            "next_cursor": "page&2",
        });

        let xml = render_response_at(&response, 10_000).unwrap();
        assert!(xml.starts_with("<mosaico>\n  <channel ref=\"#nmp/research&amp;design\">"));
        assert!(xml.contains(
            "<message from=\"@Pablo\" id=\"4e91c0\" for=\"@reviewer\" attachment-dir=\"/tmp/mosaico-files/4e91c0\" age=\"1m\">landed &lt;the&gt; commit</message>"
        ));
        assert!(xml.contains(
            "<message from=\"@reviewer\" id=\"7bc421\" time=\"6000\">approved</message>"
        ));
        assert!(xml.contains("<next cursor=\"page&amp;2\" />"));
        assert!(xml.ends_with("\n</mosaico>"));
    }

    #[test]
    fn empty_response_keeps_the_agent_native_envelope() {
        assert_eq!(
            render_response_at(&json!({"channels": [], "next_cursor": null}), 10_000).unwrap(),
            "<mosaico>\n</mosaico>"
        );
    }
}
