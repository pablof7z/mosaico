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
    pub(super) endpoint_kind: Option<crate::session_host::transport::TransportKind>,
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
    /// Hosted transport recorded at admission (`pty`/`acp`/`app-server`).
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

impl SessionStartParams {
    pub(super) fn hosted_endpoint(
        &self,
    ) -> anyhow::Result<Option<(&str, crate::session_host::transport::TransportKind)>> {
        let endpoint = self
            .pty_session
            .as_deref()
            .filter(|value| !value.is_empty());
        match (endpoint, self.endpoint_kind) {
            (Some(endpoint), Some(kind)) => Ok(Some((endpoint, kind))),
            (Some(_), None) => {
                anyhow::bail!("session_start endpoint requires explicit endpoint_kind")
            }
            (None, Some(_)) => anyhow::bail!("session_start endpoint_kind requires an endpoint"),
            (None, None) => Ok(None),
        }
    }
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
    if !matches!(transport, "" | "pty" | "acp" | "app-server") {
        anyhow::bail!("unknown admitted transport {transport:?}");
    }
    if let Some((_, endpoint_kind)) = p.hosted_endpoint()? {
        let admitted_kind = crate::session_host::transport::TransportKind::parse(transport)
            .ok_or_else(|| {
                anyhow::anyhow!("session_start endpoint requires an admitted hosted transport")
            })?;
        if admitted_kind != endpoint_kind {
            anyhow::bail!(
                "session_start endpoint_kind {} does not match admitted transport {}",
                endpoint_kind.as_str(),
                admitted_kind.as_str()
            );
        }
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

    #[test]
    fn endpoint_kind_is_typed_and_rejects_unknown_values() {
        let params: SessionStartParams = serde_json::from_value(serde_json::json!({
            "agent": "codex",
            "endpoint_kind": "acp"
        }))
        .unwrap();
        assert_eq!(
            params.endpoint_kind,
            Some(crate::session_host::transport::TransportKind::Acp)
        );
        assert!(
            serde_json::from_value::<SessionStartParams>(serde_json::json!({
                "agent": "codex",
                "endpoint_kind": "other"
            }))
            .is_err()
        );
    }

    #[test]
    fn endpoint_requires_an_explicit_matching_kind() {
        let base = serde_json::json!({
            "agent": "codex",
            "observed_harness": "codex",
            "admitted_transport": "app-server",
            "endpoint_provenance": "launch",
            "pty_session": "rpc-endpoint"
        });
        let missing: SessionStartParams = serde_json::from_value(base.clone()).unwrap();
        let error = runtime_facts(&missing).err().unwrap().to_string();
        assert!(error.contains("requires explicit endpoint_kind"), "{error}");

        let mut mismatched = base;
        mismatched["endpoint_kind"] = "acp".into();
        let mismatched: SessionStartParams = serde_json::from_value(mismatched).unwrap();
        let error = runtime_facts(&mismatched).err().unwrap().to_string();
        assert!(error.contains("does not match"), "{error}");
    }

    #[test]
    fn app_server_endpoint_kind_is_canonical() {
        let params: SessionStartParams = serde_json::from_value(serde_json::json!({
            "agent": "codex",
            "observed_harness": "codex",
            "admitted_transport": "app-server",
            "endpoint_provenance": "launch",
            "pty_session": "app-server-endpoint",
            "endpoint_kind": "app-server"
        }))
        .unwrap();
        let facts = runtime_facts(&params).unwrap();
        assert_eq!(facts.admitted_transport, "app-server");
        assert_eq!(
            params.hosted_endpoint().unwrap().unwrap().1,
            crate::session_host::transport::TransportKind::AppServer
        );
    }
}
