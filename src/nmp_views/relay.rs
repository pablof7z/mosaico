use nmp::Row;
use nostr::Event;

use crate::domain::{DomainEvent, Profile as DomainProfile, Status as DomainStatus};
use crate::fabric::nip29::wire::{Nip29WireCodec, KIND_PROFILE, KIND_STATUS};
use crate::state::{Profile, RelayEvent, Status};

use super::NmpViews;

#[derive(Clone)]
pub(super) struct ProjectedEvent {
    pub(super) event: RelayEvent,
}

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

const CHANNEL_OBSERVATION_PREFIX: &str = "mosaico-h-";

impl NmpViews {
    pub(super) fn projected_profiles(&self) -> Vec<Profile> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery.profiles;
        }
        self.observed_profiles()
            .into_iter()
            .map(|profile| profile.as_state_profile())
            .collect()
    }

    pub(super) fn projected_statuses(&self) -> Vec<Status> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery.statuses;
        }

        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .observation_rows()
            .into_iter()
            .filter_map(|(observation_id, row)| {
                let channel = observation_id.strip_prefix(CHANNEL_OBSERVATION_PREFIX)?;
                (row.row.event.kind.as_u16() == KIND_STATUS)
                    .then_some((channel.to_string(), row.row))
            })
            .filter_map(|(channel, row)| {
                let observed = observed_status_from_row(row)?;
                observed
                    .status
                    .channels
                    .iter()
                    .any(|candidate| candidate == &channel)
                    .then_some((channel, observed))
            })
            .map(|(channel, observed)| self.state_status(&observed, channel))
            .collect()
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

    pub(crate) fn observed_profile(&self, pubkey: &str) -> Option<ObservedProfile> {
        self.rows_for_kind_author(KIND_PROFILE, pubkey)
            .into_iter()
            .find_map(|row| observed_profile_from_row(row.row))
    }

    pub(crate) fn observed_profiles(&self) -> Vec<ObservedProfile> {
        self.rows_for_kind(KIND_PROFILE)
            .into_iter()
            .filter_map(|row| observed_profile_from_row(row.row))
            .collect()
    }

    pub(crate) fn observed_statuses_for_channel(&self, channel: &str) -> Vec<ObservedStatus> {
        let observation_id = format!("{CHANNEL_OBSERVATION_PREFIX}{channel}");
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .rows_for_observation(&observation_id)
            .into_iter()
            .filter(|row| row.row.event.kind.as_u16() == KIND_STATUS)
            .filter_map(|row| observed_status_from_row(row.row))
            .filter(|observed| {
                observed
                    .status
                    .channels
                    .iter()
                    .any(|candidate| candidate == channel)
            })
            .collect()
    }

    fn state_status(&self, observed: &ObservedStatus, channel_h: String) -> Status {
        let domain = &observed.status;
        let slug = (!domain.agent.slug.is_empty())
            .then(|| domain.agent.slug.clone())
            .or_else(|| {
                self.profile(&domain.agent.pubkey)
                    .map(|profile| profile.slug)
            })
            .unwrap_or_default();
        let updated_at = observed.row.event.created_at.as_secs();
        Status {
            pubkey: domain.agent.pubkey.clone(),
            channel_h,
            slug,
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

    pub(super) fn projected_events(&self) -> Vec<ProjectedEvent> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery
                .events
                .into_iter()
                .map(|event| ProjectedEvent { event })
                .collect();
        }
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .rows()
            .into_iter()
            .filter(|row| !matches!(row.row.event.kind.as_u16(), 0 | 30315 | 39000..=39002))
            .map(|row| ProjectedEvent {
                event: relay_event(&row.row.event),
            })
            .collect()
    }

    pub(super) fn projected_events_for_kind(&self, kind: u16) -> Vec<ProjectedEvent> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery
                .events
                .into_iter()
                .filter(|event| event.kind == kind as u32)
                .map(|event| ProjectedEvent { event })
                .collect();
        }
        self.rows_for_kind(kind)
            .into_iter()
            .map(|row| ProjectedEvent {
                event: relay_event(&row.row.event),
            })
            .collect()
    }

    pub(super) fn projected_events_for_kind_channel(
        &self,
        kind: u16,
        channel: &str,
    ) -> Vec<ProjectedEvent> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery
                .events
                .into_iter()
                .filter(|event| event.kind == kind as u32 && event.channel_h == channel)
                .map(|event| ProjectedEvent { event })
                .collect();
        }
        self.rows_for_kind_channel(kind, channel)
            .into_iter()
            .map(|row| ProjectedEvent {
                event: relay_event(&row.row.event),
            })
            .collect()
    }

    pub(super) fn projected_event(&self, id: &str) -> Option<ProjectedEvent> {
        #[cfg(test)]
        if let Some(delivery) = self.test_relay_delivery() {
            return delivery
                .events
                .into_iter()
                .find(|event| event.id == id)
                .map(|event| ProjectedEvent { event });
        }
        let id = nostr::EventId::from_hex(id).ok()?;
        self.row(&id).map(|row| ProjectedEvent {
            event: relay_event(&row.event),
        })
    }
}

pub(crate) fn observed_profile_from_row(row: Row) -> Option<ObservedProfile> {
    let DomainEvent::Profile(profile) = Nip29WireCodec.decode_event(&row.event)? else {
        return None;
    };
    Some(ObservedProfile { profile, row })
}

pub(crate) fn observed_status_from_row(row: Row) -> Option<ObservedStatus> {
    let DomainEvent::Status(status) = Nip29WireCodec.decode_event(&row.event)? else {
        return None;
    };
    Some(ObservedStatus { status, row })
}

impl ObservedProfile {
    pub(super) fn as_state_profile(&self) -> Profile {
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
            updated_at: self.row.event.created_at.as_secs(),
        }
    }
}

pub(super) fn relay_event(event: &Event) -> RelayEvent {
    RelayEvent {
        id: event.id.to_hex(),
        kind: event.kind.as_u16() as u32,
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs(),
        channel_h: crate::fabric::nip29::nostr_tag(event, "h")
            .unwrap_or("")
            .to_string(),
        d_tag: crate::fabric::nip29::nostr_tag(event, "d")
            .unwrap_or("")
            .to_string(),
        content: event.content.clone(),
        tags_json: serde_json::to_string(
            &event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string()),
    }
}
