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
        if provider.with_store(|s| s.is_channel_admin(ctx.channel, pk).unwrap_or(false)) {
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
    let expect_listed = provider.with_store(|s| {
        s.is_channel_member(ctx.channel, ctx.expect_member)
            .unwrap_or(false)
    });
    if !expect_already_admin && !expect_listed {
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
        sync_local_member_mirror(provider, ctx);
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

fn sync_local_member_mirror(provider: &Nip29Provider, ctx: &ChannelCtx<'_>) {
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
    // Which of the two relay-signed lists names the subject decides the row.
    // A subject on kind:39001 is an admin however the relay filled — or left
    // empty — the role position beside it.
    let role = if provider.with_store(|s| {
        s.is_channel_admin(ctx.channel, ctx.expect_member)
            .unwrap_or(false)
    }) {
        "admin"
    } else {
        "member"
    };
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
