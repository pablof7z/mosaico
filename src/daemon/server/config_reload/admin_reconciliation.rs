use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ManagedAdminTarget {
    pub(super) channel: String,
    pub(super) managed_parent: Option<String>,
    pub(super) inherited_admins: Vec<String>,
}

pub(in crate::daemon::server) fn reconcile_managed_admins(state: &Arc<DaemonState>) {
    let provider = state.provider();
    let targets = match managed_admin_targets(state, &provider) {
        Ok(targets) => targets,
        Err(error) => {
            tracing::warn!(
                error = %format!("{error:#}"),
                "could not plan managed channel administrator reconciliation"
            );
            return;
        }
    };
    tokio::spawn(async move {
        let mut failed = BTreeSet::new();
        for target in targets {
            if target
                .managed_parent
                .as_ref()
                .is_some_and(|parent| failed.contains(parent))
            {
                tracing::warn!(
                    channel = %target.channel,
                    "managed channel administrator reconciliation deferred after parent failure"
                );
                failed.insert(target.channel);
                continue;
            }
            match provider
                .reconcile_managed_admins(&target.channel, Some(&target.inherited_admins))
                .await
            {
                Ok(true) => tracing::info!(
                    channel = %target.channel,
                    "reconciled managed channel administrators for configuration generation"
                ),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        channel = %target.channel,
                        error = %error,
                        "managed channel administrator reconciliation deferred"
                    );
                    failed.insert(target.channel);
                }
            }
        }
    });
}

pub(super) fn managed_admin_targets(
    state: &DaemonState,
    provider: &Nip29Provider,
) -> Result<Vec<ManagedAdminTarget>> {
    let channels = state.with_store(|store| store.list_managed_channels())?;
    let mut pending = channels
        .into_iter()
        .map(|channel| (channel.channel_h.clone(), channel))
        .collect::<BTreeMap<_, _>>();
    let managed = pending.keys().cloned().collect::<BTreeSet<_>>();
    let mut desired = BTreeMap::<String, Vec<String>>::new();
    let mut targets = Vec::new();
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .filter(|(_, channel)| {
                channel.parent.is_empty()
                    || !managed.contains(&channel.parent)
                    || desired.contains_key(&channel.parent)
            })
            .map(|(channel, _)| channel.clone())
            .collect::<Vec<_>>();
        if ready.is_empty() {
            anyhow::bail!("managed channel ancestry contains a cycle");
        }
        for channel_h in ready {
            let channel = pending.remove(&channel_h).expect("ready channel exists");
            let managed_parent = managed
                .contains(&channel.parent)
                .then(|| channel.parent.clone());
            let inherited_admins = if channel.parent.is_empty() {
                Vec::new()
            } else if let Some(parent) = desired.get(&channel.parent) {
                parent.clone()
            } else {
                state.with_store(|store| observed_admins(store, &channel.parent))?
            };
            let channel_desired = provider
                .desired_admins(&inherited_admins)
                .context("management signing key unavailable")?;
            desired.insert(channel_h.clone(), channel_desired);
            targets.push(ManagedAdminTarget {
                channel: channel_h,
                managed_parent,
                inherited_admins,
            });
        }
    }
    Ok(targets)
}

fn observed_admins(store: &Store, channel: &str) -> Result<Vec<String>> {
    Ok(store
        .list_channel_members(channel)?
        .into_iter()
        .filter(|member| member.role == "admin")
        .map(|member| member.pubkey)
        .collect())
}
