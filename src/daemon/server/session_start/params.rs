#[derive(serde::Deserialize, Default)]
pub(super) struct SessionStartParams {
    pub(super) agent: String,
    #[serde(default)]
    pub(super) profile: Option<String>,
    /// Authoritative pubkey allocated before a managed process is spawned.
    #[serde(default)]
    pub(super) pubkey: Option<String>,
    #[serde(default)]
    pub(super) reclaimed_pubkey: Option<String>,
    #[serde(default)]
    pub(super) harness_session: Option<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    pub(super) watch_pid: Option<i32>,
    #[serde(default)]
    pub(super) pty_session: Option<String>,
    #[serde(default)]
    pub(super) endpoint_kind: Option<String>,
    #[serde(default)]
    pub(super) session_name: Option<String>,
    #[serde(default)]
    pub(super) resume_id: Option<String>,
    /// Hook adapter's asserted host. Diagnostic only.
    #[serde(default)]
    pub(super) claimed_harness: Option<String>,
    /// Harness observed from launch admission or a recognized ancestor process.
    #[serde(default)]
    pub(super) observed_harness: Option<String>,
    /// Launch-selected bundle. Empty for externally discovered sessions.
    #[serde(default)]
    pub(super) admitted_bundle: Option<String>,
    /// Hosted transport recorded at admission (`pty`/`acp`).
    #[serde(default)]
    pub(super) admitted_transport: Option<String>,
    /// Source of endpoint facts (`launch` or `hook`).
    #[serde(default)]
    pub(super) endpoint_provenance: Option<String>,
    #[serde(default)]
    pub(super) channel: Option<String>,
    #[serde(default)]
    pub(super) channels: Vec<String>,
    #[serde(default)]
    pub(super) dispatch_event: Option<String>,
}

pub(super) struct RuntimeFacts {
    pub(super) observed_harness: crate::session::Harness,
    pub(super) claimed_harness: String,
    pub(super) admitted_bundle: String,
    pub(super) admitted_transport: String,
    pub(super) endpoint_provenance: String,
}

pub(super) fn runtime_facts(p: &SessionStartParams) -> anyhow::Result<RuntimeFacts> {
    let observed = required_harness(p.observed_harness.as_deref(), "observed_harness")?;
    let claimed = optional_harness(p.claimed_harness.as_deref(), "claimed_harness")?;
    let provenance = p.endpoint_provenance.as_deref().unwrap_or("");
    if !matches!(provenance, "launch" | "hook") {
        anyhow::bail!("session_start requires endpoint_provenance launch or hook");
    }
    if provenance == "hook" && claimed.is_empty() {
        anyhow::bail!("hook session_start requires an explicit claimed_harness");
    }
    let transport = p.admitted_transport.as_deref().unwrap_or("");
    if !matches!(transport, "" | "pty" | "acp") {
        anyhow::bail!("unknown admitted transport {transport:?}");
    }
    Ok(RuntimeFacts {
        observed_harness: observed,
        claimed_harness: claimed,
        admitted_bundle: p.admitted_bundle.clone().unwrap_or_default(),
        admitted_transport: transport.to_string(),
        endpoint_provenance: provenance.to_string(),
    })
}

fn required_harness(value: Option<&str>, field: &str) -> anyhow::Result<crate::session::Harness> {
    let value = value.filter(|value| !value.is_empty()).ok_or_else(|| {
        anyhow::anyhow!("session_start requires an explicit {field}; harness guessing is forbidden")
    })?;
    let harness = crate::session::Harness::from_str(value);
    if harness == crate::session::Harness::Unknown {
        anyhow::bail!("session_start {field} {value:?} is not a recognized harness");
    }
    Ok(harness)
}

fn optional_harness(value: Option<&str>, field: &str) -> anyhow::Result<String> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(String::new());
    };
    let harness = crate::session::Harness::from_str(value);
    if harness == crate::session::Harness::Unknown {
        anyhow::bail!("session_start {field} {value:?} is not a recognized harness");
    }
    Ok(harness.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locator_shape_never_guesses_a_harness() {
        let params = SessionStartParams {
            agent: "agent".into(),
            harness_session: Some("native".into()),
            endpoint_provenance: Some("hook".into()),
            ..Default::default()
        };
        let error = runtime_facts(&params).err().unwrap().to_string();
        assert!(error.contains("explicit observed_harness"), "{error}");
    }

    #[test]
    fn claimed_and_observed_harness_remain_distinct() {
        let params = SessionStartParams {
            agent: "agent".into(),
            observed_harness: Some("grok".into()),
            claimed_harness: Some("claude-code".into()),
            admitted_transport: Some("pty".into()),
            endpoint_provenance: Some("hook".into()),
            ..Default::default()
        };
        let facts = runtime_facts(&params).unwrap();
        assert_eq!(facts.observed_harness, crate::session::Harness::Grok);
        assert_eq!(facts.claimed_harness, "claude-code");
    }
}
