use super::{ChannelCtx, Nip29Provider};
use crate::fabric::group_management::GroupMutationOutcome;
use crate::fabric::nip29::readiness::ChannelReadinessError;

pub(super) struct Outcome {
    pub(super) repaired: bool,
    pub(super) degraded: Option<ChannelReadinessError>,
}

/// Bring the relay's roster up to the invariants a ready channel must satisfy.
///
/// Every membership question is asked of the cache the retained group-records
/// observation keeps current. "Is an admin" means NAMED BY kind:39001 — not
/// that the record spelled the free-form role string "admin" — which is the
/// same reading the store itself materializes.
pub(super) async fn ensure_invariants(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    mgmt_pubkey: &str,
    parent_admins: &[String],
    admins_published_this_attempt: &[String],
) -> Outcome {
    let mut repaired = false;
    let required_admins = required_admins(
        provider,
        mgmt_pubkey,
        parent_admins,
        ctx.repair_whitelisted_admins,
    );
    let missing_admins = required_admins
        .iter()
        .filter(|pk| {
            !admins_published_this_attempt.contains(pk)
                && !provider.with_store(|s| s.is_channel_admin(ctx.channel, pk).unwrap_or(false))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !missing_admins.is_empty() {
        match published(
            provider
                .grant_admins_published(ctx.channel, &missing_admins)
                .await,
            format!(
                "one admin grant for {} users in {}",
                missing_admins.len(),
                ctx.channel
            ),
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
    let expect_listed = provider.with_store(|s| {
        s.is_channel_member(ctx.channel, ctx.expect_member)
            .unwrap_or(false)
    });
    if !expect_already_admin && !expect_listed {
        match published(
            provider
                .grant_member_published(ctx.channel, ctx.expect_member)
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
    }
    Outcome {
        repaired,
        degraded: None,
    }
}

pub(super) fn required_admins(
    provider: &Nip29Provider,
    mgmt_pubkey: &str,
    parent_admins: &[String],
    repair_whitelisted_admins: bool,
) -> Vec<String> {
    let mut required = vec![mgmt_pubkey.to_string()];
    if repair_whitelisted_admins {
        for pubkey in &provider.whitelisted_pubkeys {
            if !required.contains(pubkey) {
                required.push(pubkey.clone());
            }
        }
    }
    for pubkey in parent_admins {
        if !required.contains(pubkey) {
            required.push(pubkey.clone());
        }
    }
    required
}

fn published(outcome: GroupMutationOutcome, action: String) -> Result<(), ChannelReadinessError> {
    match outcome {
        GroupMutationOutcome::Published => Ok(()),
        GroupMutationOutcome::Failed(error) => {
            Err(ChannelReadinessError::from(error).context(action))
        }
    }
}
