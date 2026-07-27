use super::{ChannelCtx, Nip29Provider};

pub(super) fn is_ready(provider: &Nip29Provider, ctx: &ChannelCtx<'_>) -> bool {
    let Some(required_admins) = required_admins(provider, ctx) else {
        return false;
    };
    provider.with_store(|store| store_ready(store, ctx, &required_admins))
}

fn required_admins(provider: &Nip29Provider, ctx: &ChannelCtx<'_>) -> Option<Vec<String>> {
    let mut admins = vec![provider.management_pubkey()?];
    if ctx.repair_whitelisted_admins {
        for pk in &provider.whitelisted_pubkeys {
            if !admins.contains(pk) {
                admins.push(pk.clone());
            }
        }
    }
    Some(admins)
}

pub(super) fn store_ready(
    store: &crate::state::Store,
    ctx: &ChannelCtx<'_>,
    required_admins: &[String],
) -> bool {
    if store.get_channel(ctx.channel).ok().flatten().is_none() {
        return false;
    }
    let materialized_parent = store
        .channel_parent(ctx.channel)
        .ok()
        .flatten()
        .filter(|parent| !parent.is_empty());
    if ctx
        .parent_hint
        .filter(|parent| !parent.is_empty())
        .is_some()
        || materialized_parent.is_some()
    {
        return false;
    }
    if !store
        .has_channel_membership_snapshot(ctx.channel)
        .unwrap_or(false)
    {
        return false;
    }
    let member_ready = ctx.expect_member.is_empty()
        || store
            .is_channel_member(ctx.channel, ctx.expect_member)
            .unwrap_or(false);
    let admins_ready = required_admins
        .iter()
        .all(|pk| store.is_channel_admin(ctx.channel, pk).unwrap_or(false));
    member_ready && admins_ready
}
