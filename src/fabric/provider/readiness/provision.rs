use super::{ChannelCtx, Nip29Provider};
use crate::fabric::group_management::GroupPublishOutcome;
use crate::fabric::nip29::readiness::ChannelReadinessError;

pub(super) async fn missing_group(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    parent: Option<&str>,
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
    match creation {
        GroupPublishOutcome::Published => Ok(true),
        GroupPublishOutcome::Failed(error) => {
            // A concurrent creator may have won after our initial snapshot.
            // One current NMP read distinguishes that race from a real failed
            // mutation; Mosaico never retries the publication or polls its
            // derived projection.
            match provider.fetch_channel(ctx.channel).await {
                Ok(Some(_)) => Ok(false),
                Ok(None) => Err(ChannelReadinessError::from(error)
                    .context("relay metadata remained absent after group creation")),
                Err(read_error) => Err(ChannelReadinessError::reason(format!("{read_error:#}"))
                    .context("group creation failed and relay metadata could not be checked")),
            }
        }
    }
}
