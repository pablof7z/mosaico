use super::{ChannelCtx, Nip29Provider};
use crate::fabric::group_management::GroupPublishOutcome;
use crate::fabric::nip29::readiness::ChannelReadinessError;

pub(super) async fn missing_group(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    parent: Option<&str>,
    management_pubkey: &str,
) -> Result<bool, ChannelReadinessError> {
    let creation = if let Some(parent) = parent {
        let name = ctx
            .name
            .filter(|name| !name.is_empty())
            .unwrap_or(ctx.channel);
        provider
            .nip29_create_subgroup_outcome(ctx.channel, name, parent)
            .await
    } else {
        provider.nip29_create_root_outcome(ctx.channel).await
    };
    let creation_error = match creation {
        GroupPublishOutcome::Published => None,
        GroupPublishOutcome::Failed(error) => Some(error),
    };
    if creation_error.is_some() {
        if provider.fetch_and_materialize_channel(ctx.channel).await {
            return Ok(false);
        }
        return Err(creation_error
            .map(ChannelReadinessError::from)
            .unwrap_or_else(|| ChannelReadinessError::reason("group creation failed"))
            .context("relay metadata remained absent after group creation"));
    }

    if !await_metadata(provider, ctx.channel).await {
        return Err(ChannelReadinessError::reason(
            "kind:39000 did not materialize after group creation",
        ));
    }
    await_management_admin(provider, ctx.channel, management_pubkey).await;
    Ok(true)
}

async fn await_metadata(provider: &Nip29Provider, channel: &str) -> bool {
    for attempt in 0..12u32 {
        if provider.fetch_and_materialize_channel(channel).await {
            return true;
        }
        tokio::time::sleep(backoff(attempt)).await;
    }
    false
}

/// Wait for the relay's own kind:39001 to name the management identity.
///
/// Polls the cache the retained group-records observation keeps current: the
/// admin row appears the moment a host publishes the record, so this is a wait
/// on RELAY state that costs no relay read.
async fn await_management_admin(provider: &Nip29Provider, channel: &str, management_pubkey: &str) {
    for attempt in 0..6u32 {
        match provider.with_store(|s| s.is_channel_admin(channel, management_pubkey)) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => tracing::warn!(
                channel,
                attempt,
                error = %format!("{error:#}"),
                "admin state read-back failed after group creation"
            ),
        }
        tokio::time::sleep(backoff(attempt)).await;
    }
}

fn backoff(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(250 * (attempt as u64 + 1).min(3))
}
