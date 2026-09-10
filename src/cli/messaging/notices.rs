use anyhow::{Context, Result};

#[cfg(test)]
#[path = "notices/tests.rs"]
mod tests;

pub(super) fn print_send_notices(result: &serde_json::Value) -> Result<()> {
    for line in send_notice_lines(result)? {
        println!("{line}");
    }
    Ok(())
}

fn send_notice_lines(result: &serde_json::Value) -> Result<Vec<String>> {
    let Some(notices) = result.get("notices").and_then(serde_json::Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    for notice in notices {
        let summary = notice
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .context("daemon returned notice without a summary")?;
        lines.push(summary.to_string());
        if notice.get("code").and_then(serde_json::Value::as_str) != Some("untagged_agent_prefix") {
            continue;
        }
        let Some(agent) = notice
            .get("matched_agent")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let channel = result
            .get("channel")
            .and_then(serde_json::Value::as_str)
            .context("daemon response missing channel for notice command")?;
        let event_id = result
            .get("event_id")
            .and_then(serde_json::Value::as_str)
            .context("daemon response missing event_id for notice command")?;
        let short_id = crate::util::short_id(event_id);
        let message = format!("That message, {short_id}, was for you; I forgot to tag you.");
        lines.push(format!(
            "To tag that agent now, run: `mosaico channel send --channel {} --tag {} --message {}`",
            shell_quote(channel),
            shell_quote(agent),
            shell_quote(&message),
        ));
    }
    Ok(lines)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
