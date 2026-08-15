use super::*;

pub(super) fn validate_runtime_facts(
    registration: &RegisterSession,
    facts: &AdmittedRuntimeFacts,
) -> Result<()> {
    let observed = facts.observed_harness.trim();
    if observed.is_empty() {
        anyhow::bail!("runtime facts require observed_harness");
    }
    let harness = crate::session::Harness::from_str(observed);
    if harness == crate::session::Harness::Unknown || harness.as_str() != observed {
        anyhow::bail!("runtime facts contain unknown observed_harness {observed:?}");
    }
    if registration.observed_harness != observed {
        anyhow::bail!(
            "registration observed_harness {:?} does not match admitted facts {observed:?}",
            registration.observed_harness
        );
    }
    if !matches!(
        facts.transport.as_str(),
        "" | "pty" | "acp" | "app-server" | "pi-rpc"
    ) {
        anyhow::bail!(
            "runtime facts contain unknown transport {:?}",
            facts.transport
        );
    }
    match facts.endpoint_provenance.as_str() {
        "launch" => validate_launch(facts),
        "hook" => validate_hook(facts),
        provenance => anyhow::bail!(
            "runtime facts require endpoint_provenance launch or hook, got {provenance:?}"
        ),
    }
}

fn validate_launch(facts: &AdmittedRuntimeFacts) -> Result<()> {
    if !facts.claimed_harness.is_empty() {
        anyhow::bail!("launch runtime facts forbid claimed_harness");
    }
    if facts.transport.is_empty() {
        anyhow::bail!("launch runtime facts require transport");
    }
    Ok(())
}

fn validate_hook(facts: &AdmittedRuntimeFacts) -> Result<()> {
    let claimed = facts.claimed_harness.trim();
    if claimed.is_empty() {
        anyhow::bail!("hook runtime facts require claimed_harness");
    }
    let claimed_harness = crate::session::Harness::from_str(claimed);
    if claimed_harness == crate::session::Harness::Unknown || claimed_harness.as_str() != claimed {
        anyhow::bail!("runtime facts contain unknown claimed_harness {claimed:?}");
    }
    if !facts.preset.is_empty() {
        anyhow::bail!("hook runtime facts forbid preset");
    }
    Ok(())
}
