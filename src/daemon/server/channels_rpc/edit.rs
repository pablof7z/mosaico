//! `channel_edit`: set a channel's durable `about` and read the relay back.

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
    let event_id = state.nmp.publish_group(&channel_h, builder, &mgmt_keys)?;
    let confirmed = wait_for_channel_about(state, &channel_h, &p.about).await;
    let channel = state
        .with_store(|store| super::channel_resolve::channel_reference_for(store, &channel_h))?;
    if !confirmed {
        anyhow::bail!("relay did not confirm updated about for channel {channel}");
    }

    Ok(serde_json::json!({
        "event_id": event_id.to_hex(),
        "channel": channel,
        "about": p.about,
        "confirmed": confirmed,
    }))
}

async fn wait_for_channel_about(state: &Arc<DaemonState>, channel_h: &str, about: &str) -> bool {
    for _ in 0..20 {
        state
            .provider
            .fetch_and_materialize_channel(channel_h)
            .await;
        let matches = state.with_store(|s| {
            s.get_channel(channel_h)
                .ok()
                .flatten()
                .map(|c| c.about)
                .as_deref()
                == Some(about)
        });
        if matches {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    false
}
