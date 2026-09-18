//! Derive host capability deltas from semantic changes, not profile timestamps.

use std::collections::BTreeMap;

use super::capture::{AgentCap, HostCap, ViewInputs};
use super::model::{AgentRow, FabricView, HostRow};

pub(crate) fn inject_host_deltas(
    view: &mut FabricView,
    previous: &ViewInputs,
    current: &ViewInputs,
) {
    let before = previous
        .meta
        .hosts
        .iter()
        .map(|host| (host.name.as_str(), host))
        .collect::<BTreeMap<_, _>>();
    let mut changed = Vec::new();

    for host in &current.meta.hosts {
        let prior = before.get(host.name.as_str()).copied();
        let agents = changed_agents(prior, host);
        let roots_changed = prior.is_some_and(|old| old.roots != host.roots);
        if prior.is_none() || roots_changed || !agents.is_empty() {
            changed.push(HostRow {
                name: host.name.clone(),
                roots: host.roots.clone(),
                agents,
            });
        }
    }

    view.hosts = (!changed.is_empty()).then_some(changed);
}

fn changed_agents(previous: Option<&HostCap>, current: &HostCap) -> Vec<AgentRow> {
    let before = previous
        .map(|host| {
            host.agents
                .iter()
                .map(|agent| (agent.reference.as_str(), agent))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    current
        .agents
        .iter()
        .filter(|agent| capability_changed(before.get(agent.reference.as_str()).copied(), agent))
        .map(|agent| AgentRow {
            reference: agent.reference.clone(),
            about: crate::agent_about::for_injection(&agent.about),
        })
        .collect()
}

fn capability_changed(previous: Option<&AgentCap>, current: &AgentCap) -> bool {
    previous.is_none_or(|agent| {
        crate::agent_about::for_injection(&agent.about)
            != crate::agent_about::for_injection(&current.about)
    })
}
