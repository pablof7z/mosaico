use super::{ensure_channel_ready_inner, ChannelCtx, ChannelGate, Nip29Provider};
use crate::fabric::nip29::readiness::ChannelReadinessError;

pub(in crate::fabric::provider) fn stored_parent_hint(
    provider: &Nip29Provider,
    channel: &str,
) -> anyhow::Result<Option<String>> {
    resolved_parent_hint(provider, channel, None)
}

pub(super) fn resolved_parent_hint(
    provider: &Nip29Provider,
    channel: &str,
    caller_hint: Option<&str>,
) -> anyhow::Result<Option<String>> {
    provider.with_store(|store| resolved_parent_hint_from_store(store, channel, caller_hint))
}

pub(super) fn resolved_parent_hint_from_store(
    store: &crate::state::Store,
    channel: &str,
    caller_hint: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let relay_parent = store.channel_parent(channel)?;
    let pending_parent = store
        .channel_resolution_parent(channel)?
        .or(store.session_readiness_parent(channel)?)
        .or_else(|| caller_hint.map(str::to_string));
    Ok(crate::fabric::nip29::readiness::effective_parent_hint(
        relay_parent,
        pending_parent.as_deref(),
        channel,
    ))
}

pub(super) async fn ensure_parent(
    provider: &Nip29Provider,
    child: &ChannelCtx<'_>,
    parent: &str,
    management_pubkey: &str,
) -> Result<Vec<String>, ChannelReadinessError> {
    let grandparent = stored_parent_hint(provider, parent).map_err(|error| {
        ChannelReadinessError::reason(format!("{error:#}"))
            .context(format!("reading pending ancestry for {parent} failed"))
    })?;
    let parent_ctx = ChannelCtx {
        channel: parent,
        expect_member: management_pubkey,
        parent_hint: grandparent.as_deref(),
        name: None,
        repair_whitelisted_admins: child.repair_whitelisted_admins,
    };
    if let ChannelGate::Degraded(error) = ensure_channel_ready_inner(provider, parent_ctx).await {
        return Err(error.context(format!("parent channel {parent} readiness failed")));
    }
    Ok(provider.with_store(|store| {
        store
            .list_channel_members(parent)
            .unwrap_or_default()
            .into_iter()
            .filter(|member| member.role == "admin")
            .map(|member| member.pubkey)
            .collect()
    }))
}
