//! Product projections from the complete current NIP-29 view.

use crate::domain::DomainEvent;

use super::{RelayEvent, Status, Store};

impl Store {
    pub(super) fn group_projection(&self) -> crate::group_projection::GroupProjection {
        #[cfg(test)]
        if let Some(delivery) = self
            .test_snapshots
            .read()
            .expect("test snapshots poisoned")
            .groups
            .as_ref()
        {
            return delivery.projection();
        }
        self.nip29_view
            .as_ref()
            .map(|view| crate::group_projection::GroupProjection::new(&view.group_snapshots()))
            .unwrap_or_else(|| crate::group_projection::GroupProjection::new(&[]))
    }

    /// One projection of the current view for a caller that would otherwise
    /// rebuild it once per lookup.
    pub(crate) fn group_projection_snapshot(&self) -> crate::group_projection::GroupProjection {
        self.group_projection()
    }

    pub(super) fn relay_events(&self) -> Vec<RelayEvent> {
        #[cfg(test)]
        if let Some(delivery) = self
            .test_snapshots
            .read()
            .expect("test snapshots poisoned")
            .relay
            .as_ref()
        {
            return delivery.events.clone();
        }
        self.nip29_view
            .as_ref()
            .into_iter()
            .flat_map(|view| view.content_events())
            .map(|event| relay_event(&event))
            .collect()
    }

    pub(super) fn statuses(&self) -> Vec<Status> {
        #[cfg(test)]
        if let Some(delivery) = self
            .test_snapshots
            .read()
            .expect("test snapshots poisoned")
            .relay
            .as_ref()
        {
            return delivery.statuses.clone();
        }
        self.nip29_view
            .as_ref()
            .into_iter()
            .flat_map(|view| view.content_events())
            .filter_map(|event| {
                let DomainEvent::Status(status) =
                    crate::fabric::nip29::wire::Nip29WireCodec.decode_event(&event)?
                else {
                    return None;
                };
                Some((status, event.created_at.as_secs()))
            })
            .flat_map(|(status, updated_at)| {
                status.channels.into_iter().map(move |channel_h| Status {
                    pubkey: status.agent.pubkey.clone(),
                    channel_h,
                    slug: status.agent.slug.clone(),
                    title: status.title.clone(),
                    activity: status.activity.clone(),
                    workspace: status.workspace.clone(),
                    branch: status.branch.clone(),
                    state: status.state,
                    state_since: status.state_since,
                    last_seen: updated_at,
                    updated_at,
                    expiration: status.expires_at.unwrap_or(0),
                })
            })
            .collect()
    }
}

pub(super) fn relay_event(event: &nostr::prelude::Event) -> RelayEvent {
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
