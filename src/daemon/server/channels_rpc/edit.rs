//! `channel_edit`: publish a channel's durable `about` through NMP.

use super::*;

pub(in crate::daemon::server) async fn rpc_channel_edit(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        about: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_edit params")?;
    crate::channel_about::validate_channel_about(&p.about)?;
    // Operator TUI and agent CLI both edit via the management key. A session
    // anchor is optional; when present it only identifies the caller for logs.
    let _ = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    );
    let channel_h = resolve_target_channel(state, &p.channel)?;

    let mgmt_keys = state.management_keys()?;
    let builder = as_nostr(nmp_nip29::edit_metadata(nmp_nip29::GroupMetadataEdit {
        about: Some(p.about.clone()),
        ..nmp_nip29::GroupMetadataEdit::default()
    }));
    let event_id = state
        .snapshot()
        .nmp
        .publish_group_and_wait(&channel_h, builder, &mgmt_keys)
        .await
        .context("publishing channel description")?;
    let channel = state
        .with_store(|store| super::channel_resolve::channel_reference_for(store, &channel_h))?;

    Ok(serde_json::json!({
        "event_id": event_id.to_hex(),
        "channel": channel,
        "about": p.about,
        "confirmed": true,
    }))
}
