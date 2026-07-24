use super::Nip29Provider;
use anyhow::{Context, Result};
use nostr::Event;
use std::collections::BTreeSet;
use std::time::Duration;

const RELATIONSHIP_READBACK_TIMEOUT: Duration = Duration::from_secs(15);

impl Nip29Provider {
    fn observe_group_metadata(&self, parent_h: &str) -> Result<nmp::Subscription> {
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        self.nmp.observe(&crate::reconcile::SubscriptionQuery {
            kinds: BTreeSet::from([KIND_GROUP_METADATA]),
            authors: BTreeSet::new(),
            tag: Some(('d', parent_h.to_string())),
        })
    }

    /// Wait until the relay's parent metadata reciprocally confirms `child_h`.
    ///
    /// Croissant derives this reverse projection from the accepted child 9007;
    /// clients only verify relay truth and never race replacement-style parent
    /// metadata writes of their own.
    pub(in crate::fabric::provider) async fn confirm_parent_lists_child(
        &self,
        parent_h: &str,
        child_h: &str,
    ) -> Result<()> {
        let subscription = self
            .observe_group_metadata(parent_h)
            .with_context(|| format!("observing parent {parent_h:?} metadata"))?;
        let child_h = child_h.to_string();
        tokio::task::spawn_blocking(move || {
            let deadline = std::time::Instant::now() + RELATIONSHIP_READBACK_TIMEOUT;
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    anyhow::bail!("relay did not confirm child {child_h:?} in parent metadata");
                }
                let frame = subscription
                    .recv_timeout(remaining)
                    .context("parent metadata observation disconnected")?;
                if frame
                    .deltas
                    .iter()
                    .filter_map(|delta| delta.event())
                    .any(|event| children_from_metadata(event).contains(&child_h))
                {
                    return Ok(());
                }
            }
        })
        .await
        .context("joining parent metadata observation")?
    }
}

fn children_from_metadata(event: &Event) -> BTreeSet<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("child"))
                .then(|| values.get(1).cloned())
                .flatten()
                .filter(|child| !child.is_empty())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn child_parser_preserves_every_existing_relationship() {
        let event = EventBuilder::new(Kind::from(39000u16), "")
            .tags([
                Tag::parse(["d", "parent"]).unwrap(),
                Tag::parse(["child", "first"]).unwrap(),
                Tag::parse(["name", "Parent"]).unwrap(),
                Tag::parse(["child", "second"]).unwrap(),
                Tag::parse(["child", "first"]).unwrap(),
                Tag::parse(["child", ""]).unwrap(),
            ])
            .sign_with_keys(&Keys::generate())
            .unwrap();

        assert_eq!(
            children_from_metadata(&event),
            BTreeSet::from(["first".to_string(), "second".to_string()])
        );
    }
}
