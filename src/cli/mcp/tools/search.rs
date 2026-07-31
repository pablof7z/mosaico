use anyhow::{Context, Result};
use serde_json::{json, Value};

pub(super) async fn call(args: &Value, _caller: Option<&str>) -> Result<Value> {
    let from = string_array(args, "from")?;
    let to = string_array(args, "to")?;
    let contains = string_array(args, "contains")?;
    let channels = string_array(args, "channels")?;
    let since = time_arg(args, "since")?;
    let until = time_arg(args, "until")?;
    let limit = args.get("limit").and_then(Value::as_u64);
    let cursor = super::opt_string(args, "cursor");
    ensure_cursor_only(
        cursor.as_deref(),
        &from,
        &to,
        &contains,
        &channels,
        since,
        until,
        limit,
    )?;
    let params = json!({
        "from": from,
        "to": to,
        "contains": contains,
        "channels": channels,
        "since": since,
        "until": until,
        "limit": limit,
        "cursor": cursor,
    });
    let structured = crate::cli::search::daemon_search(params).await?;
    tool_result(structured)
}

#[allow(clippy::too_many_arguments)]
fn ensure_cursor_only(
    cursor: Option<&str>,
    from: &[String],
    to: &[String],
    contains: &[String],
    channels: &[String],
    since: Option<u64>,
    until: Option<u64>,
    limit: Option<u64>,
) -> Result<()> {
    if cursor.is_some()
        && (!from.is_empty()
            || !to.is_empty()
            || !contains.is_empty()
            || !channels.is_empty()
            || since.is_some()
            || until.is_some()
            || limit.is_some())
    {
        anyhow::bail!("cursor must be used alone; it already contains the normalized search query");
    }
    Ok(())
}

fn tool_result(structured: Value) -> Result<Value> {
    let text = crate::cli::search::render_response(&structured)?;
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": structured,
        "isError": false,
    }))
}

fn string_array(args: &Value, key: &str) -> Result<Vec<String>> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .with_context(|| format!("{key} must be an array of strings"))?
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|text| !text.trim().is_empty())
                .map(ToString::to_string)
                .with_context(|| format!("{key} entries must be non-empty strings"))
        })
        .collect()
}

fn time_arg(args: &Value, key: &str) -> Result<Option<u64>> {
    args.get(key)
        .map(|value| {
            value.as_u64().map(Ok).unwrap_or_else(|| {
                value
                    .as_str()
                    .with_context(|| format!("{key} must be a Unix timestamp or duration"))
                    .and_then(|raw| {
                        crate::cli::search::parse_search_time(raw).map_err(anyhow::Error::msg)
                    })
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_arrays_are_typed_and_times_accept_unix_values() {
        let args = json!({
            "from": ["@Pablo", "@reviewer"],
            "channels": ["#nmp", "#"],
            "since": 1785348600,
        });
        assert_eq!(
            string_array(&args, "from").unwrap(),
            ["@Pablo", "@reviewer"]
        );
        assert_eq!(string_array(&args, "channels").unwrap(), ["#nmp", "#"]);
        assert_eq!(time_arg(&args, "since").unwrap(), Some(1_785_348_600));
    }

    #[test]
    fn search_arrays_reject_scalar_and_empty_entries() {
        assert!(string_array(&json!({"from": "@Pablo"}), "from").is_err());
        assert!(string_array(&json!({"to": [""]}), "to").is_err());
    }

    #[test]
    fn continuation_cursor_must_be_used_without_filters() {
        assert!(ensure_cursor_only(
            Some("opaque"),
            &[],
            &[],
            &["commit".into()],
            &[],
            None,
            None,
            None,
        )
        .unwrap_err()
        .to_string()
        .contains("used alone"));
        ensure_cursor_only(Some("opaque"), &[], &[], &[], &[], None, None, None).unwrap();
    }

    #[test]
    fn tool_text_is_the_same_xml_as_the_cli_and_structured_content_is_preserved() {
        let structured = json!({
            "channels": [{
                "ref": "#nmp/research",
                "messages": [{
                    "event_id": "4e91c0b7f2de",
                    "from": "Pablo",
                    "recipients": ["reviewer"],
                    "body": "landed",
                    "created_at": 1,
                }]
            }],
            "next_cursor": "opaque",
        });
        let result = tool_result(structured.clone()).unwrap();

        assert_eq!(result["structuredContent"], structured);
        assert_eq!(
            result["content"][0]["text"],
            crate::cli::search::render_response(&structured).unwrap()
        );
        assert_eq!(result["isError"], false);
    }
}
