use anyhow::{bail, Context as _, Result};
use std::io::IsTerminal as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentKeyStatus {
    Absent,
    Ready,
    Missing,
}

impl AgentKeyStatus {
    fn from_response(value: &serde_json::Value) -> Result<Self> {
        match value["status"].as_str() {
            Some("absent") => Ok(Self::Absent),
            Some("ready") => Ok(Self::Ready),
            Some("missing") => Ok(Self::Missing),
            Some(status) => bail!("agent_key_status returned unknown status {status:?}"),
            None => bail!("agent_key_status response missing status"),
        }
    }
}

pub(super) async fn ensure_ready(slug: &str) -> Result<()> {
    if !crate::identity::is_valid_slug(slug) {
        return Ok(());
    }
    let value =
        crate::cli::daemon_call_async("agent_key_status", serde_json::json!({ "slug": slug }))
            .await
            .with_context(|| format!("checking persistent identity for agent {slug:?}"))?;
    let status = AgentKeyStatus::from_response(&value)?;
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let create = decide_repair(slug, status, interactive, |prompt| {
        dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(true)
            .interact()
            .map_err(Into::into)
    })?;
    if !create {
        return Ok(());
    }

    let value =
        crate::cli::daemon_call_async("agent_key_create", serde_json::json!({ "slug": slug }))
            .await
            .with_context(|| format!("creating persistent identity for agent {slug:?}"))?;
    let created = value["created"]
        .as_bool()
        .context("agent_key_create response missing created")?;
    if created {
        eprintln!("Created and persisted a key for agent {slug:?}.");
    }
    Ok(())
}

fn decide_repair(
    slug: &str,
    status: AgentKeyStatus,
    interactive: bool,
    confirm: impl FnOnce(&str) -> Result<bool>,
) -> Result<bool> {
    if status != AgentKeyStatus::Missing {
        return Ok(false);
    }
    if !interactive {
        bail!(
            "agent {slug:?} uses perSessionKey:false but has no persisted key; \
             run this launch in a terminal to create one"
        );
    }
    let prompt =
        format!("Persistent identity for agent {slug:?} has no key. Create and persist one now?");
    if !confirm(&prompt)? {
        bail!("persistent key for agent {slug:?} was not created; launch cancelled");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_confirmation_requests_key_creation() {
        let mut prompt = String::new();
        let create = decide_repair("chief-of-staff", AgentKeyStatus::Missing, true, |message| {
            prompt = message.to_string();
            Ok(true)
        })
        .unwrap();

        assert!(create);
        assert!(prompt.contains("chief-of-staff"));
        assert!(prompt.contains("Create and persist"));
    }

    #[test]
    fn declining_stops_launch_without_requesting_creation() {
        let error = decide_repair("chief-of-staff", AgentKeyStatus::Missing, true, |_| {
            Ok(false)
        })
        .unwrap_err();

        assert!(error.to_string().contains("was not created"));
    }

    #[test]
    fn non_interactive_launch_never_creates_identity_material() {
        let error = decide_repair("chief-of-staff", AgentKeyStatus::Missing, false, |_| {
            panic!("non-interactive launch must not prompt")
        })
        .unwrap_err();

        assert!(error.to_string().contains("run this launch in a terminal"));
    }

    #[test]
    fn absent_and_ready_records_need_no_prompt() {
        for status in [AgentKeyStatus::Absent, AgentKeyStatus::Ready] {
            assert!(!decide_repair("writer", status, false, |_| {
                panic!("healthy launch must not prompt")
            })
            .unwrap());
        }
    }
}
