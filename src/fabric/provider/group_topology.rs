use super::Nip29Provider;
use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::{Event, Filter};
use std::collections::BTreeSet;
use std::time::Duration;

const RELATIONSHIP_READBACK_ATTEMPTS: u32 = 6;

impl Nip29Provider {
    async fn try_fetch_group_children(&self, parent_h: &str) -> Result<Option<BTreeSet<String>>> {
        use crate::fabric::nip29::wire::{kind, KIND_GROUP_METADATA};

        let filter = Filter::new()
            .kind(kind(KIND_GROUP_METADATA))
            .identifier(parent_h);
        let events = self
            .transport
            .fetch(filter, Duration::from_secs(5))
            .await
            .with_context(|| {
                format!("fetching parent {parent_h:?} kind:39000 child relationships")
            })?;
        Ok(events
            .iter()
            .max_by_key(|event| event.created_at.as_secs())
            .map(children_from_metadata))
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
        let mut last_observation = "parent metadata was absent".to_string();
        for attempt in 0..RELATIONSHIP_READBACK_ATTEMPTS {
            match self.try_fetch_group_children(parent_h).await {
                Ok(Some(observed)) if observed.contains(child_h) => return Ok(()),
                Ok(Some(observed)) => {
                    last_observation = format!(
                        "parent metadata listed {} child relationship(s), but not {child_h:?}",
                        observed.len()
                    );
                }
                Ok(None) => {
                    last_observation = "parent metadata was absent".to_string();
                }
                Err(error) => {
                    last_observation = format!("parent metadata readback failed: {error:#}");
                }
            }
            if attempt + 1 < RELATIONSHIP_READBACK_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(250 * (u64::from(attempt) + 1).min(3)))
                    .await;
            }
        }

        bail!("relay did not confirm parent {parent_h:?} child {child_h:?}: {last_observation}")
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
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

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
