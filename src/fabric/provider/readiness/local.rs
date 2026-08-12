use super::{ChannelCtx, Nip29Provider};
use std::collections::BTreeSet;

pub(super) async fn is_ready(provider: &Nip29Provider, ctx: &ChannelCtx<'_>) -> bool {
    let Some(required_admins) = required_admins(provider) else {
        return false;
    };
    let Some(snapshot) = provider.current_group_snapshot(ctx.channel) else {
        return false;
    };
    let managed = provider
        .with_store(|store| store.is_managed_channel(ctx.channel))
        .unwrap_or(false);
    snapshot_ready(&snapshot, ctx, &required_admins, managed)
}

fn required_admins(provider: &Nip29Provider) -> Option<Vec<String>> {
    let mut admins = vec![provider.management_pubkey()?];
    for pk in &provider.whitelisted_pubkeys {
        if !admins.contains(pk) {
            admins.push(pk.clone());
        }
    }
    Some(admins)
}

pub(super) fn snapshot_ready(
    snapshot: &nmp::nip29::GroupSnapshot,
    ctx: &ChannelCtx<'_>,
    required_admins: &[String],
    managed: bool,
) -> bool {
    let groups = crate::nmp_views::GroupProjection::new(std::slice::from_ref(snapshot));
    if groups.get_channel(ctx.channel).is_none() {
        return false;
    }
    let observed_parent = groups
        .channel_parent(ctx.channel)
        .filter(|parent| !parent.is_empty());
    if ctx
        .parent_hint
        .filter(|parent| !parent.is_empty())
        .is_some()
        || observed_parent.is_some()
    {
        return false;
    }
    if !groups.group_state_available(ctx.channel) {
        return false;
    }
    let member_ready =
        ctx.expect_member.is_empty() || groups.is_channel_member(ctx.channel, ctx.expect_member);
    let current_admins = groups
        .list_channel_members(ctx.channel)
        .into_iter()
        .filter(|member| member.role == "admin")
        .map(|member| member.pubkey)
        .collect::<BTreeSet<_>>();
    let required_admins = required_admins.iter().cloned().collect::<BTreeSet<_>>();
    let admins_ready = if managed {
        current_admins == required_admins
    } else {
        required_admins.is_subset(&current_admins)
    };
    member_ready && admins_ready
}
