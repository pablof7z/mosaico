use super::Nip29Provider;
use anyhow::Context;
use anyhow::Result;
use std::collections::BTreeSet;
use std::time::Duration;

const RELATIONSHIP_READBACK_TIMEOUT: Duration = Duration::from_secs(15);

impl Nip29Provider {
    async fn fetch_parent_children(&self, parent_h: &str) -> Result<BTreeSet<String>> {
        let snapshot = self.group_snapshot(parent_h).await?;
        Ok(snapshot
            .metadata
            .as_ref()
            .map(|metadata| {
                metadata
                    .tags
                    .iter()
                    .filter_map(|tag| {
                        (tag.first().map(String::as_str) == Some("child"))
                            .then(|| tag.get(1).cloned())
                            .flatten()
                            .filter(|child| !child.is_empty())
                    })
                    .collect()
            })
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
        let mut changes = self.view.subscribe_group_changes();
        let readback = async {
            loop {
                match self.fetch_parent_children(parent_h).await {
                    Ok(children) if children.contains(child_h) => return Ok(()),
                    Ok(_) => {}
                    Err(error) => tracing::debug!(
                        parent = parent_h,
                        child = child_h,
                        error = %format!("{error:#}"),
                        "parent relationship read awaits a newer complete view"
                    ),
                }
                changes
                    .changed()
                    .await
                    .context("NIP-29 view closed during parent relationship readback")?;
            }
        };
        tokio::time::timeout(RELATIONSHIP_READBACK_TIMEOUT, readback)
            .await
            .with_context(|| {
                format!("relay did not confirm child {child_h:?} in parent metadata")
            })?
    }
}
