use super::*;

pub(in crate::daemon::server) async fn rpc_channel_archive(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_archive params")?;
    let _rec = resolve_caller(state, params, "channel archive")?;
    let channel = resolve_target_channel(state, &p.channel)?;

    archive_channel(state, &channel).await
}

pub(in crate::daemon::server) async fn archive_channel(
    state: &Arc<DaemonState>,
    channel: &str,
) -> Result<serde_json::Value> {
    let channel_ref = state
        .with_store(|store| super::super::channel_resolve::channel_reference_for(store, channel))?;
    let delivered = state.provider().group_snapshot(channel).await?;
    let snapshots = [delivered];
    let groups = crate::nmp_views::GroupProjection::new(&snapshots);
    let current = groups
        .get_channel(channel)
        .with_context(|| format!("resolved channel {channel_ref} has no metadata row"))?;
    let archived_about = crate::state::archived_channel_about(&current.about);

    let (event_id, metadata_confirmed) = if current.about == archived_about {
        (String::new(), true)
    } else {
        let mgmt_keys = state.management_keys()?;
        let builder = crate::fabric::nip29::lifecycle::as_nostr(nmp_nip29::edit_metadata(
            nmp_nip29::GroupMetadataEdit {
                about: Some(archived_about.clone()),
                ..nmp_nip29::GroupMetadataEdit::default()
            },
        ));
        let event_id = state
            .nmp()
            .publish_group_and_wait(channel, builder, &mgmt_keys)
            .await
            .context("publishing archived channel metadata")?;
        (event_id.to_hex(), true)
    };

    let members = groups.list_channel_members(channel);
    let admins = members.iter().filter(|m| m.role == "admin").count();
    let remove_targets = archive_removal_targets(&members);
    if !remove_targets.is_empty() {
        let outcome = state
            .provider()
            .remove_members_published(channel, &remove_targets)
            .await;
        outcome.require_published(format!(
            "removing {} non-admin member(s) in one event while archiving {}",
            remove_targets.len(),
            channel_ref
        ))?;
    }

    Ok(serde_json::json!({
        "channel": channel_ref,
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
