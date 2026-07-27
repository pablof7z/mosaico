use super::{ChannelCtx, Nip29Provider};
use crate::fabric::group_management::GroupMutationOutcome;
use crate::fabric::nip29::readiness::ChannelReadinessError;
use std::collections::{HashMap, HashSet};

pub(super) struct Outcome {
    pub(super) repaired: bool,
    pub(super) degraded: Option<ChannelReadinessError>,
}

pub(super) async fn ensure_invariants(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    mgmt_pubkey: &str,
    parent_admins: &[String],
    roles: &HashMap<String, String>,
    members: &HashSet<String>,
) -> Outcome {
    let mut repaired = false;
    let mut required_admins: Vec<String> = vec![mgmt_pubkey.to_string()];
    if ctx.repair_whitelisted_admins {
        required_admins.extend(provider.whitelisted_pubkeys.iter().cloned());
    }
    for pk in parent_admins {
        if !required_admins.contains(pk) {
            required_admins.push(pk.clone());
        }
    }
    for pk in &required_admins {
        if roles.get(pk.as_str()).map(String::as_str) == Some("admin") {
            continue;
        }
        match confirmed(
            provider.grant_admin_confirmed(ctx.channel, pk).await,
            format!("admin grant for {pk} in {}", ctx.channel),
        ) {
            Ok(()) => repaired = true,
            Err(error) => {
                return Outcome {
                    repaired,
                    degraded: Some(error),
                };
            }
        }
    }
    if ctx.expect_member.is_empty() {
        return Outcome {
            repaired,
            degraded: None,
        };
    }

    let expect_already_admin = mgmt_pubkey == ctx.expect_member
        || provider
            .whitelisted_pubkeys
            .iter()
            .any(|pk| pk == ctx.expect_member)
        || parent_admins.iter().any(|pk| pk == ctx.expect_member);
    if !expect_already_admin
        && !members.contains(ctx.expect_member)
        && !roles.contains_key(ctx.expect_member)
    {
        match confirmed(
            provider
                .grant_member_confirmed(ctx.channel, ctx.expect_member)
                .await,
            format!("member grant for {} in {}", ctx.expect_member, ctx.channel),
        ) {
            Ok(()) => repaired = true,
            Err(error) => {
                return Outcome {
                    repaired,
                    degraded: Some(error),
                };
            }
        }
    } else {
        sync_local_member_mirror(provider, ctx, roles);
    }
    Outcome {
        repaired,
        degraded: None,
    }
}

fn confirmed(outcome: GroupMutationOutcome, action: String) -> Result<(), ChannelReadinessError> {
    match outcome {
        GroupMutationOutcome::Confirmed => Ok(()),
        GroupMutationOutcome::Unconfirmed { detail } => Err(ChannelReadinessError::reason(
            format!("{action} was not confirmed: {detail}"),
        )),
        GroupMutationOutcome::Failed(error) => {
            Err(ChannelReadinessError::from(error).context(action))
        }
    }
}

fn sync_local_member_mirror(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    roles: &HashMap<String, String>,
) {
    let locally =
        provider.with_store(
            |s| match s.is_channel_member(ctx.channel, ctx.expect_member) {
                Ok(present) => present,
                Err(e) => {
                    tracing::error!(
                        channel = ctx.channel,
                        pubkey = ctx.expect_member,
                        error = %e,
                        "ensure_channel_ready: is_channel_member probe failed; re-syncing"
                    );
                    false
                }
            },
        );
    if locally {
        return;
    }
    let role = roles
        .get(ctx.expect_member)
        .map(String::as_str)
        .unwrap_or("member");
    provider.with_store(|s| {
        if let Err(e) = s.upsert_channel_member(
            ctx.channel,
            ctx.expect_member,
            role,
            crate::util::now_secs(),
        ) {
            tracing::error!(
                channel = ctx.channel,
                pubkey = ctx.expect_member,
                error = %e,
                "ensure_channel_ready: local member mirror sync failed"
            );
        }
    });
}
