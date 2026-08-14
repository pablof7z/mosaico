use anyhow::{Context, Result};
use nmp::{ReceiptResult, ReceiptStream, RelayState, WriteOutcome};
use nostr::{EventBuilder, EventId, Keys, PublicKey};

use super::NmpHost;

impl NmpHost {
    /// Publish one group-management draft and wait for NMP's durable terminal
    /// result. Success means every configured group host explicitly published
    /// the event; local custody alone is never promoted to relay success.
    pub(crate) async fn publish_group_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        let (event_id, result) = self.publish_group_result(group, builder, keys).await?;
        require_every_group_host_published(&result)?;
        Ok(event_id)
    }

    /// Publish one group draft and return NMP's unreduced per-relay terminal
    /// truth so callers can preserve every relay outcome.
    pub(crate) async fn publish_group_result(
        self: &std::sync::Arc<Self>,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<(EventId, ReceiptResult)> {
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            let stream = host.publish_groups_receipt([group], builder, &keys)?;
            await_group_publication(stream)
        })
        .await
        .context("joining NMP group-publication result")?
    }

    /// Add every named user in one kind:9000 event and await one result.
    pub(crate) async fn add_group_users_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        users: Vec<nmp::nip29::GroupUser>,
        keys: &Keys,
    ) -> Result<EventId> {
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            host.ensure_identity(&keys)?;
            #[cfg(test)]
            if let Some(refusal) = host.test_io.take_write() {
                refusal?;
            }
            let scope = nmp::nip29::on(host.relays.iter().cloned())
                .map_err(|error| anyhow::anyhow!("no configured NIP-29 group host: {error:?}"))?;
            let stream = scope
                .group(group)
                .add_users(&host.engine, keys.public_key(), users)
                .map_err(|error| anyhow::anyhow!("publishing NIP-29 add-users: {error}"))?;
            finish_group_publication(stream)
        })
        .await
        .context("joining NMP add-users result")?
    }

    /// Remove every named user in one kind:9001 event and await one result.
    pub(crate) async fn remove_group_users_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        pubkeys: Vec<PublicKey>,
        keys: &Keys,
    ) -> Result<EventId> {
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            host.ensure_identity(&keys)?;
            #[cfg(test)]
            if let Some(refusal) = host.test_io.take_write() {
                refusal?;
            }
            let scope = nmp::nip29::on(host.relays.iter().cloned())
                .map_err(|error| anyhow::anyhow!("no configured NIP-29 group host: {error:?}"))?;
            let stream = scope
                .group(group)
                .remove_users(&host.engine, keys.public_key(), pubkeys)
                .map_err(|error| anyhow::anyhow!("publishing NIP-29 remove-users: {error}"))?;
            finish_group_publication(stream)
        })
        .await
        .context("joining NMP remove-users result")?
    }
}

fn finish_group_publication(stream: ReceiptStream) -> Result<EventId> {
    let (event_id, result) = await_group_publication(stream)?;
    require_every_group_host_published(&result)?;
    Ok(event_id)
}

fn await_group_publication(stream: ReceiptStream) -> Result<(EventId, ReceiptResult)> {
    let event_id = stream.event_id;
    let result = stream
        .result()
        .context("awaiting NMP's terminal group-publication result")?;
    Ok((event_id, result))
}

pub(super) fn require_every_group_host_published(result: &ReceiptResult) -> Result<()> {
    if result.outcome != WriteOutcome::Settled {
        anyhow::bail!("NIP-29 group publication ended as {:?}", result.outcome);
    }
    if result.relays.is_empty() {
        anyhow::bail!("NIP-29 group publication settled without a relay result");
    }
    let failures = result
        .relays
        .iter()
        .filter_map(|(relay, state)| match state {
            RelayState::Published => None,
            RelayState::Rejected { reason } => {
                Some(format!("{relay} rejected the event: {reason}"))
            }
            RelayState::AuthFailed { reason, .. } => {
                Some(format!("{relay} refused authentication: {reason}"))
            }
            RelayState::GaveUp => Some(format!("{relay} exhausted delivery attempts")),
            RelayState::Waiting(_) | RelayState::Sent { .. } => {
                Some(format!("{relay} ended with nonterminal state {state:?}"))
            }
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("NIP-29 group publication failed: {}", failures.join("; "))
    }
}
