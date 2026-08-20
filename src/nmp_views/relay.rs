use std::collections::BTreeSet;

use nmp::Row;
use nostr::EventId;

use crate::domain::{DomainEvent, Profile as DomainProfile, Status as DomainStatus};
use crate::fabric::nip29::wire::{Nip29WireCodec, KIND_STATUS};
use crate::state::{Profile, RelayEvent, Status};

use super::NmpViews;

/// Complete Mosaico profile meaning together with the exact NMP row that
/// supplied it. Keeping the row preserves event identity and relay sources;
/// the narrower [`Profile`] value exists only for established product callers.
#[derive(Clone)]
pub(crate) struct ObservedProfile {
    pub(crate) profile: DomainProfile,
    pub(crate) row: Row,
}

/// Complete Mosaico status meaning together with the exact NMP row that
/// supplied it. The domain value deliberately remains one multi-channel
/// replaceable event rather than a synthetic row per `h` tag.
#[derive(Clone)]
pub(crate) struct ObservedStatus {
    pub(crate) status: DomainStatus,
    pub(crate) row: Row,
}

impl NmpViews {
    pub(super) fn projected_statuses(&self) -> Vec<Status> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery.statuses;
        }

        // Under the single broad `#h` observation (mosaico#837) a kind:30315
        // status arrives once and names one or more channels in its own `#h`
        // tags; the projection fans it out to every named channel. The channel
        // comes from the decoded status, not the observation id. Dedup by
        // (row id, channel) so a status owned by more than one observation
        // still projects once per channel.
        let mut seen: BTreeSet<(EventId, String)> = BTreeSet::new();
        let mut out = Vec::new();
        for (_observation_id, row) in self
            .rows
            .lock()
            .expect("NMP row views poisoned")
            .observation_rows()
        {
            if row.row.kind().as_u16() != KIND_STATUS {
                continue;
            }
            let Some(observed) = observed_status_from_row(row.row) else {
                continue;
            };
            for channel in &observed.status.channels {
                if !seen.insert((observed.row.id(), channel.clone())) {
                    continue;
                }
                out.push(self.state_status(&observed, channel.clone()));
            }
        }
        out
    }

    pub(super) fn projected_statuses_for_channel(&self, channel: &str) -> Vec<Status> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery
                .statuses
                .into_iter()
                .filter(|status| status.channel_h == channel)
                .collect();
        }

        self.observed_statuses_for_channel(channel)
            .into_iter()
            .map(|observed| self.state_status(&observed, channel.to_string()))
            .collect()
    }

    pub(crate) fn observed_statuses_for_channel(&self, channel: &str) -> Vec<ObservedStatus> {
        // Fan out by the decoded `status.channels` (mosaico#837): a status is
        // in this channel when the channel it names matches, regardless of
        // which observation owns the row. Dedup by row id so a status owned by
        // several observations is returned once.
        let mut seen: BTreeSet<EventId> = BTreeSet::new();
        let mut out = Vec::new();
        for (_observation_id, row) in self
            .rows
            .lock()
            .expect("NMP row views poisoned")
            .observation_rows()
        {
            if row.row.kind().as_u16() != KIND_STATUS {
                continue;
            }
            let Some(observed) = observed_status_from_row(row.row) else {
                continue;
            };
            if !observed
                .status
                .channels
                .iter()
                .any(|candidate| candidate == channel)
            {
                continue;
            }
            if !seen.insert(observed.row.id()) {
                continue;
            }
            out.push(observed);
        }
        out
    }

    fn state_status(&self, observed: &ObservedStatus, channel_h: String) -> Status {
        let domain = &observed.status;
        let updated_at = observed.row.created_at().as_secs();
        // The slug comes from the status event itself; a session that omitted the
        // `slug` tag is enriched from its kind:0 profile by the Store read path
        // (where the retained profile feed is reachable), not here — NmpViews no
        // longer reads kind:0 from the mirror (mosaico#837).
        Status {
            pubkey: domain.agent.pubkey.clone(),
            channel_h,
            slug: domain.agent.slug.clone(),
            title: domain.title.clone(),
            activity: domain.activity.clone(),
            workspace: domain.workspace.clone(),
            branch: domain.branch.clone(),
            state: domain.state,
            state_since: domain.state_since,
            last_seen: updated_at,
            updated_at,
            expiration: domain.expires_at.unwrap_or(0),
        }
    }
}

pub(crate) fn observed_profile_from_row(row: Row) -> Option<ObservedProfile> {
    let event = row.signed_event()?;
    let DomainEvent::Profile(profile) = Nip29WireCodec.decode_event(&event)? else {
        return None;
    };
    Some(ObservedProfile { profile, row })
}

pub(crate) fn observed_status_from_row(row: Row) -> Option<ObservedStatus> {
    let event = row.signed_event()?;
    let DomainEvent::Status(status) = Nip29WireCodec.decode_event(&event)? else {
        return None;
    };
    Some(ObservedStatus { status, row })
}

impl ObservedProfile {
    pub(crate) fn as_state_profile(&self) -> Profile {
        let name = self.profile.agent.slug.clone();
        Profile {
            pubkey: self.profile.agent.pubkey.clone(),
            name: name.clone(),
            slug: name,
            agent_slug: self.profile.agent_slug.clone(),
            host: self.profile.host.clone(),
            is_backend: self.profile.is_backend,
            agents: self.profile.agents.clone(),
            workspaces: self.profile.workspaces.clone(),
            updated_at: self.row.created_at().as_secs(),
        }
    }
}

pub(super) fn relay_event(row: &Row) -> RelayEvent {
    RelayEvent {
        id: row.id().to_hex(),
        kind: row.kind().as_u16() as u32,
        pubkey: row.pubkey().to_hex(),
        created_at: row.created_at().as_secs(),
        channel_h: crate::fabric::nip29::nostr_tag(row.tags(), "h")
            .unwrap_or("")
            .to_string(),
        d_tag: crate::fabric::nip29::nostr_tag(row.tags(), "d")
            .unwrap_or("")
            .to_string(),
        content: row.content().to_string(),
        tags_json: serde_json::to_string(
            &row.tags()
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::collections::BTreeSet;

    fn signed(kind: u16, content: &str, tags: Vec<Tag>) -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    #[test]
    fn relay_event_extracts_every_field_from_a_row() {
        let event = signed(
            9,
            "hello",
            vec![
                Tag::parse(["h", "room"]).unwrap(),
                Tag::parse(["d", "dval"]).unwrap(),
                Tag::parse(["p", "pk-mention"]).unwrap(),
            ],
        );
        let row = Row::from_relay_event(event.clone(), BTreeSet::new());
        let decoded = relay_event(&row);

        assert_eq!(decoded.id, event.id.to_hex());
        assert_eq!(decoded.kind, 9);
        assert_eq!(decoded.pubkey, event.pubkey.to_hex());
        assert_eq!(decoded.created_at, event.created_at.as_secs());
        assert_eq!(decoded.channel_h, "room");
        assert_eq!(decoded.d_tag, "dval");
        assert_eq!(decoded.content, "hello");
        // tags_json round-trips the full tag set, not just h/d.
        let parsed: Vec<Vec<String>> = serde_json::from_str(&decoded.tags_json).unwrap();
        let has = |label: &str| parsed.iter().any(|t| t.first().is_some_and(|f| f == label));
        assert!(has("h") && has("d") && has("p"));
    }

    #[test]
    fn relay_event_defaults_missing_h_and_d_tags_to_empty() {
        let event = signed(7, "+1", vec![Tag::parse(["e", "parent"]).unwrap()]);
        let row = Row::from_relay_event(event, BTreeSet::new());
        let decoded = relay_event(&row);
        assert_eq!(decoded.channel_h, "", "no h tag → empty channel_h");
        assert_eq!(decoded.d_tag, "", "no d tag → empty d_tag");
        assert_eq!(decoded.kind, 7);
        assert_eq!(decoded.content, "+1");
    }
}
