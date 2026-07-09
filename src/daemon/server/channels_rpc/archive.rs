use super::*;

pub(in crate::daemon::server) async fn rpc_channels_archive(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_archive params")?;
    let rec = resolve_caller(state, params, "channels archive")?;
    let channel = match resolve_target_channel(state, &rec, &p.channel)? {
        TargetChannel::Unique(h) => h,
        TargetChannel::Ambiguous(v) => return Ok(v),
    };

    archive_channel(state, &channel).await
}

pub(in crate::daemon::server) async fn archive_channel(
    state: &Arc<DaemonState>,
    channel: &str,
) -> Result<serde_json::Value> {
    let current = state
        .with_store(|s| s.get_channel(channel))?
        .with_context(|| format!("resolved channel {channel:?} has no metadata row"))?;
    let archived_about = crate::state::archived_channel_about(&current.about);

    let event_id = if current.about == archived_about {
        String::new()
    } else {
        let mgmt_keys = state.management_keys()?;
        let builder =
            crate::fabric::nip29::lifecycle::group_edit_metadata(channel, &archived_about)?;
        state
            .transport
            .publish_signed(builder, &mgmt_keys)
            .await?
            .to_hex()
    };
    let _ = state.provider.fetch_and_materialize_channel(channel).await;
    let metadata_confirmed = state.with_store(|s| s.is_archived_channel(channel))?;

    refresh_channel_members_cache(state, channel).await;
    let members = state.with_store(|s| s.list_channel_members(channel))?;
    let admins = members.iter().filter(|m| m.role == "admin").count();
    let remove_targets = archive_removal_targets(&members);
    let mut failures = Vec::new();
    for pubkey in &remove_targets {
        let outcome = state
            .provider
            .remove_member_confirmed(channel, pubkey)
            .await;
        if !outcome.is_confirmed() {
            failures.push(format!("{}:{outcome:?}", crate::util::pubkey_short(pubkey)));
        }
    }
    if !failures.is_empty() {
        anyhow::bail!(
            "archived metadata for {channel}, but failed to confirm removal of {} non-admin member(s): {}",
            failures.len(),
            failures.join(", ")
        );
    }

    Ok(serde_json::json!({
        "channel": channel,
        "about": archived_about,
        "event_id": event_id,
        "metadata_confirmed": metadata_confirmed,
        "removed_members": remove_targets.len(),
        "admins_remaining": admins,
    }))
}

fn archive_removal_targets(members: &[crate::state::ChannelMember]) -> Vec<String> {
    members
        .iter()
        .filter(|m| m.role != "admin")
        .map(|m| m.pubkey.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests;
