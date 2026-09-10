use anyhow::{Context, Result};

#[cfg(test)]
#[path = "notices/tests.rs"]
mod tests;

pub(super) fn print_send_coaching(result: &serde_json::Value) -> Result<()> {
    for line in send_coaching_lines(result)? {
        println!("{line}");
    }
    Ok(())
}

fn send_coaching_lines(result: &serde_json::Value) -> Result<Vec<String>> {
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
        let channel = notice
            .get("channel")
            .and_then(serde_json::Value::as_str)
            .context("daemon returned untagged_agent_prefix notice without channel")?;
        let id = result
            .get("id")
            .and_then(serde_json::Value::as_str)
            .context("daemon response missing id for coaching command")?;
        let message = format!("That message, {id}, was for you; I forgot to tag you.");
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
