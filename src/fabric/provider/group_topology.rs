use super::Nip29Provider;
use anyhow::Result;
use nostr::Event;
use std::collections::BTreeSet;
use std::time::Duration;

const RELATIONSHIP_READBACK_TIMEOUT: Duration = Duration::from_secs(15);

impl Nip29Provider {
    async fn fetch_parent_children(&self, parent_h: &str) -> Result<BTreeSet<String>> {
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        let read = self
            .nmp
            .fetch_group_records(parent_h, 10, Duration::from_secs(5))
            .await?;
        super::group_state::require_proven_projection(&read, "reading parent relationships")?;
        Ok(read
            .rows
            .iter()
            .map(|row| &row.event)
            .filter(|event| event.kind.as_u16() == KIND_GROUP_METADATA)
            .max_by_key(|event| event.created_at.as_secs())
            .map(children_from_metadata)
            .unwrap_or_default())
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
        let deadline = tokio::time::Instant::now() + RELATIONSHIP_READBACK_TIMEOUT;
        loop {
            let last_error = match self.fetch_parent_children(parent_h).await {
                Ok(children) if children.contains(child_h) => return Ok(()),
                Ok(_) => None,
                Err(error) => Some(format!("{error:#}")),
            };
            if tokio::time::Instant::now() >= deadline {
                if let Some(error) = last_error {
                    anyhow::bail!(
                        "relay did not confirm child {child_h:?} in parent metadata; \
                         final read failed: {error}"
                    );
                }
                anyhow::bail!("relay did not confirm child {child_h:?} in parent metadata");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
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
