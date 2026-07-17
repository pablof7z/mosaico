use super::Nip29Provider;
use anyhow::Result;
use nostr_sdk::prelude::{EventId, Kind, Tag};

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

impl Nip29Provider {
    /// Publish one backend capability advertisement (`kind:30555`) signed by the
    /// daemon management key. The `d` tag is the capability slug; every root
    /// channel where this backend offers it appears as an `h` tag.
    pub(crate) async fn publish_agent_roster(
        &self,
        slug: &str,
        host: &str,
        use_criteria: &str,
        root_channels: &[String],
    ) -> Result<EventId> {
        let mgmt_keys = self
            .management_keys()
            .ok_or_else(|| anyhow::anyhow!("no signing key (mosaicoPrivateKey) set"))?;
        let mut tags = vec![tag(&["d", slug])?, tag(&["hostname", host])?];
        if !use_criteria.trim().is_empty() {
            tags.push(tag(&["use-criteria", use_criteria.trim()])?);
        }
        let mut roots = root_channels
            .iter()
            .map(|h| h.trim())
            .filter(|h| !h.is_empty())
            .collect::<Vec<_>>();
        roots.sort_unstable();
        roots.dedup();
        for root in roots {
            tags.push(tag(&["h", root])?);
        }

        let builder = nostr_sdk::prelude::EventBuilder::new(
            Kind::from(crate::fabric::nip29::wire::KIND_AGENT_ROSTER),
            "",
        )
        .tags(tags);
        let signed = self.nmp.sign_event(builder, &mgmt_keys).await?;
        let event_id = self.nmp.enqueue_group_event(&signed)?;
        self.with_store(|s| {
            crate::fabric::nip29::materializer::Nip29Materializer::materialize_agent_roster(
                s, &signed,
            )
        });
        Ok(event_id)
    }
}
