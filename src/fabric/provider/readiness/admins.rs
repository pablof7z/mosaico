use std::collections::BTreeSet;

use super::{verify, Nip29Provider};
use crate::fabric::group_management::GroupMutationOutcome;
use crate::fabric::nip29::readiness::ChannelReadinessError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Policy {
    /// Mosaico can prove ownership, so configured and inherited admins are the
    /// exact desired set.
    Exact,
    /// An observed group without ownership proof may receive required grants,
    /// but Mosaico never revokes authority from it.
    Additive,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Delta {
    pub(super) additions: Vec<String>,
    pub(super) removals: Vec<String>,
}

pub(super) struct Applied {
    pub(super) changed: bool,
    pub(super) removed: Vec<String>,
}

pub(super) fn delta(current: &[String], required: &[String], policy: Policy) -> Delta {
    let current = current.iter().cloned().collect::<BTreeSet<_>>();
    let required = required.iter().cloned().collect::<BTreeSet<_>>();
    let additions = required.difference(&current).cloned().collect();
    let removals = match policy {
        Policy::Exact => current.difference(&required).cloned().collect(),
        Policy::Additive => Vec::new(),
    };
    Delta {
        additions,
        removals,
    }
}

pub(super) async fn apply(
    provider: &Nip29Provider,
    channel: &str,
    required: &[String],
    policy: Policy,
    already_added: &[String],
) -> Result<Applied, ChannelReadinessError> {
    let current = provider.with_store(|store| {
        store.list_channel_members(channel).map(|members| {
            members
                .into_iter()
                .filter(|member| member.role == "admin")
                .map(|member| member.pubkey)
                .collect::<Vec<_>>()
        })
    });
    let current = current.map_err(|error| {
        ChannelReadinessError::reason(format!("reading administrators failed: {error:#}"))
    })?;
    let mut delta = delta(&current, required, policy);
    delta
        .additions
        .retain(|pubkey| !already_added.contains(pubkey));
    let mut changed = false;
    if !delta.additions.is_empty() {
        published(
            provider
                .grant_admins_published(channel, &delta.additions)
                .await,
            format!(
                "one admin grant for {} users in {channel}",
                delta.additions.len()
            ),
        )?;
        changed = true;
    }
    if !delta.removals.is_empty() {
        published(
            provider
                .remove_members_published(channel, &delta.removals)
                .await,
            format!(
                "one obsolete admin removal for {} users in {channel}",
                delta.removals.len()
            ),
        )?;
        changed = true;
    }
    Ok(Applied {
        changed,
        removed: delta.removals,
    })
}

pub(super) fn published(
    outcome: GroupMutationOutcome,
    action: String,
) -> Result<(), ChannelReadinessError> {
    match outcome {
        GroupMutationOutcome::Published => Ok(()),
        GroupMutationOutcome::Failed(error) => {
            Err(ChannelReadinessError::from(error).context(action))
        }
    }
}

impl Nip29Provider {
    /// Reconcile one provably managed group's exact administrator set. The
    /// caller may supply a parent generation already planned in memory; absent
    /// that, the currently observed parent roster is used once.
    pub(crate) async fn reconcile_managed_admins(
        &self,
        channel: &str,
        inherited_admins: Option<&[String]>,
    ) -> Result<bool, ChannelReadinessError> {
        let managed = self.with_store(|store| store.is_managed_channel(channel));
        if !managed.map_err(|error| {
            ChannelReadinessError::reason(format!("reading channel ownership failed: {error:#}"))
        })? {
            return Ok(false);
        }
        if !self.admin_list_observed(channel) {
            return Err(ChannelReadinessError::reason(
                "relay-signed admin list has not been observed yet",
            ));
        }
        let management = self
            .management_pubkey()
            .ok_or_else(|| ChannelReadinessError::reason("management signing key unavailable"))?;
        let parent_admins = match inherited_admins {
            Some(admins) => admins.to_vec(),
            None => self.observed_parent_admins(channel)?,
        };
        let required = verify::required_admins(self, &management, &parent_admins);
        apply(self, channel, &required, Policy::Exact, &[])
            .await
            .map(|applied| applied.changed)
    }

    pub(crate) fn desired_admins(&self, inherited_admins: &[String]) -> Option<Vec<String>> {
        self.management_pubkey()
            .map(|management| verify::required_admins(self, &management, inherited_admins))
    }

    fn observed_parent_admins(&self, channel: &str) -> Result<Vec<String>, ChannelReadinessError> {
        let parent = self
            .with_store(|store| store.channel_parent(channel))
            .map_err(|error| {
                ChannelReadinessError::reason(format!("reading channel parent failed: {error:#}"))
            })?;
        let Some(parent) = parent.filter(|parent| !parent.is_empty()) else {
            return Ok(Vec::new());
        };
        if !self.admin_list_observed(&parent) {
            return Err(ChannelReadinessError::reason(format!(
                "relay-signed parent admin list for {parent} has not been observed yet"
            )));
        }
        self.with_store(|store| {
            store.list_channel_members(&parent).map(|members| {
                members
                    .into_iter()
                    .filter(|member| member.role == "admin")
                    .map(|member| member.pubkey)
                    .collect()
            })
        })
        .map_err(|error| {
            ChannelReadinessError::reason(format!(
                "reading parent administrators failed: {error:#}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn managed_admins_are_an_exact_set() {
        assert_eq!(
            delta(
                &strings(&["management", "a", "b"]),
                &strings(&["management", "a"]),
                Policy::Exact,
            ),
            Delta {
                additions: Vec::new(),
                removals: strings(&["b"]),
            }
        );
    }

    #[test]
    fn inherited_parent_admin_is_retained_while_new_admins_are_batched() {
        assert_eq!(
            delta(
                &strings(&["management", "old"]),
                &strings(&["management", "configured", "parent"]),
                Policy::Exact,
            ),
            Delta {
                additions: strings(&["configured", "parent"]),
                removals: strings(&["old"]),
            }
        );
    }

    #[test]
    fn unrelated_observed_group_is_never_subtracted() {
        assert_eq!(
            delta(
                &strings(&["management", "relay-owner"]),
                &strings(&["management"]),
                Policy::Additive,
            ),
            Delta {
                additions: Vec::new(),
                removals: Vec::new(),
            }
        );
    }
}
