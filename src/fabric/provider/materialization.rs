use super::Nip29Provider;
use crate::fabric::nip29::{nostr_tag, wire};
use crate::fabric::{MaterializationOutcome, ProjectionProvenance, RawEnvelope};
use crate::state::Store;
use anyhow::{Context, Result};

impl Nip29Provider {
    /// Decode one raw envelope and apply all store side-effects.
    pub(crate) fn materialize_observed(
        &self,
        event: &nostr::Event,
        provenance: &ProjectionProvenance,
        store: &Store,
    ) -> MaterializationOutcome {
        let outcome = crate::fabric::materialize_observed(event, provenance, store);
        let env = RawEnvelope::Nostr(event.clone());
        if let Some(channel) = roster_snapshot_channel(&env) {
            self.readiness.invalidate_channel(channel);
        }
        outcome
    }

    /// Apply one row returned by a bounded NMP read through the same
    /// provenance reducer as the retained observation stream.
    pub(super) fn materialize_bounded_row(
        &self,
        observation_id: &str,
        row: &nmp::Row,
        evidence: &[nmp::AcquisitionEvidence],
    ) -> Result<MaterializationOutcome> {
        let generation = self.nmp.allocate_projection_generation();
        let evidence_json = crate::nmp_host::scoped_evidence_json(evidence)?;
        let relay_settled = crate::nmp_host::relay_settled(evidence);
        let sources_json = serde_json::to_string(
            &row.sources
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )?;
        self.with_store(|store| {
            anyhow::ensure!(
                store.begin_projection_frame(
                    observation_id,
                    generation,
                    &evidence_json,
                    relay_settled,
                )?,
                "stale bounded projection generation"
            );
            store.claim_projection_event(
                observation_id,
                generation,
                &row.event.id.to_hex(),
                &sources_json,
            )?;
            let outcome = self.materialize_observed(
                &row.event,
                &ProjectionProvenance {
                    source_event_id: row.event.id.to_hex(),
                },
                store,
            );
            if relay_settled {
                for event_id in store.settle_projection_frame(observation_id, generation)? {
                    store
                        .retract_projection_source(&event_id)
                        .with_context(|| {
                            format!("retracting stale bounded projection source {event_id}")
                        })?;
                }
            }
            Ok(outcome)
        })
    }
}

fn roster_snapshot_channel(env: &RawEnvelope) -> Option<&str> {
    let RawEnvelope::Nostr(event) = env;
    match event.kind.as_u16() {
        wire::KIND_GROUP_ADMINS | wire::KIND_GROUP_MEMBERS => nostr_tag(event, "d"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event(kind: u16, tags: Vec<Tag>) -> RawEnvelope {
        RawEnvelope::Nostr(
            EventBuilder::new(Kind::from(kind), "")
                .tags(tags)
                .sign_with_keys(&Keys::generate())
                .unwrap(),
        )
    }

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    #[test]
    fn roster_snapshots_identify_readiness_invalidation_channel() {
        let admins = event(wire::KIND_GROUP_ADMINS, vec![tag(&["d", "chan"])]);
        let members = event(wire::KIND_GROUP_MEMBERS, vec![tag(&["d", "chan"])]);
        let metadata = event(wire::KIND_GROUP_METADATA, vec![tag(&["d", "chan"])]);
        let chat = event(wire::KIND_CHAT, vec![tag(&["h", "chan"])]);

        assert_eq!(roster_snapshot_channel(&admins), Some("chan"));
        assert_eq!(roster_snapshot_channel(&members), Some("chan"));
        assert_eq!(roster_snapshot_channel(&metadata), None);
        assert_eq!(roster_snapshot_channel(&chat), None);
    }
}
