use anyhow::{Context, Result};

#[cfg(test)]
#[path = "notices/tests.rs"]
mod tests;

pub(super) fn print_recipient_reminders(result: &serde_json::Value) -> Result<()> {
    for reminder in recipient_reminders(result)? {
        println!("{reminder}");
    }
    Ok(())
}

pub(super) fn print_send_coaching(result: &serde_json::Value) -> Result<()> {
    for line in send_coaching_lines(result)? {
        println!("{line}");
    }
    Ok(())
}

fn recipient_reminders(result: &serde_json::Value) -> Result<Vec<&str>> {
    result
        .get("recipient_reminders")
        .and_then(serde_json::Value::as_array)
        .context("daemon response missing recipient_reminders")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .context("daemon returned a non-string recipient reminder")
        })
        .collect()
}

fn send_coaching_lines(result: &serde_json::Value) -> Result<Vec<String>> {
    let notices = result
        .get("coaching")
        .and_then(serde_json::Value::as_array)
        .context("daemon response missing coaching notices")?;
    let mut lines = Vec::new();
    for notice in notices {
        let summary = notice
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .context("daemon returned coaching without a summary")?;
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
            .context("daemon response missing channel for coaching command")?;
        let event_id = result
            .get("event_id")
            .and_then(serde_json::Value::as_str)
            .context("daemon response missing event_id for coaching command")?;
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
