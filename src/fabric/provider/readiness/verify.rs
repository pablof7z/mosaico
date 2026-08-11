use super::{admins, ChannelCtx, Nip29Provider};
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
