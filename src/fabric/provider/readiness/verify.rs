use super::{admins, ChannelCtx, Nip29Provider};
use crate::fabric::nip29::readiness::ChannelReadinessError;

pub(super) struct Outcome {
    pub(super) repaired: bool,
    pub(super) degraded: Option<ChannelReadinessError>,
}

/// Complete the known initial roster after NMP reports terminal success for a
/// create+lock operation. The relay makes the creator an administrator, so the
/// remaining desired administrators and optional member can be expressed as
/// at most two typed, batched mutations without reading a projected echo back.
pub(super) async fn initialize_created_group(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    mgmt_pubkey: &str,
    parent_admins: &[String],
) -> Result<(), ChannelReadinessError> {
    let required = required_admins(provider, mgmt_pubkey, parent_admins);
    let additional_admins = required
        .iter()
        .filter(|pubkey| pubkey.as_str() != mgmt_pubkey)
        .cloned()
        .collect::<Vec<_>>();
    if !additional_admins.is_empty() {
        admins::published(
            provider
                .grant_admins_published(ctx.channel, &additional_admins)
                .await,
            format!(
                "initial admin grant for {} users in {}",
                additional_admins.len(),
                ctx.channel
            ),
        )?;
    }
    if !ctx.expect_member.is_empty() && !required.iter().any(|key| key == ctx.expect_member) {
        admins::published(
            provider
                .grant_member_published(ctx.channel, ctx.expect_member)
                .await,
            format!("initial member grant in {}", ctx.channel),
        )?;
    }
    Ok(())
}

/// Bring the relay's roster up to the invariants a ready channel must satisfy.
///
/// Every membership question is asked of NMP's complete group snapshot. "Is an
/// admin" means named by kind:39001, not that a record used a free-form role
/// string.
pub(super) async fn ensure_invariants(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    mgmt_pubkey: &str,
    parent_admins: &[String],
    admins_published_this_attempt: &[String],
) -> Outcome {
    let mut repaired = false;
    let required_admins = required_admins(provider, mgmt_pubkey, parent_admins);
    let policy =
        if provider.with_store(|store| store.is_managed_channel(ctx.channel).unwrap_or(false)) {
            admins::Policy::Exact
        } else {
            admins::Policy::Additive
        };
    let applied = match admins::apply(
        provider,
        ctx.channel,
        &required_admins,
        policy,
        admins_published_this_attempt,
    )
    .await
    {
        Ok(applied) => applied,
        Err(error) => {
            return Outcome {
                repaired,
                degraded: Some(error),
            };
        }
    };
    repaired |= applied.changed;
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
    let expect_removed_as_admin = applied.removed.iter().any(|pk| pk == ctx.expect_member);
    let expect_listed = !expect_removed_as_admin
        && provider.with_store(|s| {
            s.is_channel_member(ctx.channel, ctx.expect_member)
                .unwrap_or(false)
        });
    if !expect_already_admin && !expect_listed {
        match admins::published(
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
) -> Vec<String> {
    let mut required = vec![mgmt_pubkey.to_string()];
    for pubkey in &provider.whitelisted_pubkeys {
        if !required.contains(pubkey) {
            required.push(pubkey.clone());
        }
    }
    for pubkey in parent_admins {
        if !required.contains(pubkey) {
            required.push(pubkey.clone());
        }
    }
    required
}
