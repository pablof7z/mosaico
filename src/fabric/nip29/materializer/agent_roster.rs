use crate::state::{AgentRoster, Store};
use nostr_sdk::Event;

pub(super) fn materialize(store: &Store, event: &Event) {
    let Some(slug) = super::super::nostr_tag(event, "d") else {
        return;
    };
    let host = super::super::nostr_tag(event, "hostname")
        .or_else(|| super::super::nostr_tag(event, "host"))
        .unwrap_or("");
    let use_criteria = super::super::nostr_tag(event, "use-criteria").unwrap_or("");
    let backend_pubkey = event.pubkey.to_hex();
    let advertised_channels = collect_tag_values(event, "h");
    let had_h_tags = !advertised_channels.is_empty();
    let channels: Vec<String> = advertised_channels
        .into_iter()
        .filter(|channel_h| {
            store.is_root_channel(channel_h).unwrap_or(false)
                && store
                    .is_channel_admin(channel_h, &backend_pubkey)
                    .unwrap_or(false)
        })
        .collect();
    if channels.is_empty() && had_h_tags {
        return;
    }
    let roster = AgentRoster {
        backend_pubkey,
        host: host.to_string(),
        slug: slug.to_string(),
        use_criteria: use_criteria.to_string(),
        channels,
        updated_at: event.created_at.as_secs(),
    };
    if let Err(e) = store.replace_agent_roster(&roster) {
        tracing::error!(
            backend = %roster.backend_pubkey,
            slug = %roster.slug,
            error = %e,
            "materialize_agent_roster: relay_agent_roster replace failed"
        );
    }
}

fn collect_tag_values(event: &Event, tag_name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            (s.first().map(String::as_str) == Some(tag_name))
                .then(|| s.get(1).cloned())
                .flatten()
        })
        .collect()
}
