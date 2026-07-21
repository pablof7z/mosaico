use anyhow::{bail, Result};

#[derive(Default)]
pub(super) struct Attempt {
    pub(super) actions: Vec<String>,
    pub(super) error: Option<anyhow::Error>,
}

pub(super) async fn run() -> Attempt {
    let mut actions = Vec::new();
    let error = apply(&mut actions).await.err();
    Attempt { actions, error }
}

async fn apply(actions: &mut Vec<String>) -> Result<()> {
    repair_config(actions)?;
    repair_skill(actions)?;
    repair_selected_integrations(actions)?;
    super::super::daemon_lifecycle::restart().await?;
    actions.push("restarted the daemon without terminating PTY supervisors".to_string());
    Ok(())
}

fn repair_config(actions: &mut Vec<String>) -> Result<()> {
    use super::super::install::ConfigRepair;

    match super::super::install::repair_device_config()? {
        ConfigRepair::Unchanged => {}
        ConfigRepair::Created => actions.push(format!(
            "created baseline device config at {} with no trusted operators",
            crate::config::config_path().display()
        )),
        ConfigRepair::GeneratedManagementKey => actions.push(format!(
            "generated missing mosaicoPrivateKey in {}",
            crate::config::config_path().display()
        )),
    }
    Ok(())
}

fn repair_skill(actions: &mut Vec<String>) -> Result<()> {
    use super::super::install::SkillHealthState;

    let before = super::super::install::skill_health()?;
    let unhealthy = before
        .targets
        .iter()
        .filter(|target| target.state != SkillHealthState::Healthy)
        .map(|target| (target.label, target.path.clone()))
        .collect::<Vec<_>>();
    if unhealthy.is_empty() {
        return Ok(());
    }
    let after = super::super::install::repair_skill()?;
    if let Some(target) = after
        .targets
        .iter()
        .find(|target| target.state != SkillHealthState::Healthy)
    {
        bail!(
            "runtime skill target {} remains {:?} at {}",
            target.label,
            target.state,
            target.path.display()
        );
    }
    actions.extend(unhealthy.into_iter().map(|(label, path)| {
        format!(
            "restored {label} runtime skill target at {}",
            path.display()
        )
    }));
    Ok(())
}

fn repair_selected_integrations(actions: &mut Vec<String>) -> Result<()> {
    for harness in super::super::install::harnesses()?
        .into_iter()
        .filter(|harness| harness.detected)
    {
        let present = super::super::install::is_present(&harness);
        let healthy = super::super::install::is_installed(&harness);
        if !present || healthy {
            continue;
        }
        super::super::install::repair_integration(&harness)?;
        if !super::super::install::is_installed(&harness) {
            bail!(
                "{} integration remains unhealthy at {}",
                harness.display,
                harness.config_path.display()
            );
        }
        actions.push(format!(
            "repaired {} integration at {}",
            harness.display,
            harness.config_path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "repair/tests.rs"]
mod tests;
