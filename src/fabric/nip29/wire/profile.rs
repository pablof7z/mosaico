use crate::domain::{AgentRef, DomainEvent, Profile};
use anyhow::Result;
use nostr::{Event, EventBuilder};

use super::{all_tag_values, first_tag, has_bare_tag, kind, tag, KIND_PROFILE};

pub(super) fn encode(pf: &Profile) -> Result<EventBuilder> {
    let name = if pf.is_backend {
        pf.agent.slug.clone()
    } else {
        crate::idref::session_handle_from_profile_name(&pf.agent.slug, &pf.agent_slug)
    };
    let content = serde_json::json!({ "name": name }).to_string();
    let mut tags = vec![tag(&["host", &pf.host])?];
    if !pf.agent_slug.is_empty() {
        tags.push(tag(&["agent-slug", &pf.agent_slug])?);
    }
    if !pf.is_backend && !pf.workspace.is_empty() {
        tags.push(tag(&["workspace", &pf.workspace])?);
    }
    for owner in &pf.owners {
        tags.push(tag(&["p", owner])?);
    }
    if pf.is_backend {
        tags.push(tag(&["backend"])?);
        // Advertise the managed-agent inventory so clients (e.g. 29er iOS) can offer
        // an add-agent picker: on tap they send `add <slug>`, so slug stays
        // command-compatible. `desc` carries the agent's compact use criteria.
        for (slug, desc) in &pf.agents {
            tags.push(tag(&["agent", slug, desc])?);
        }
        for workspace in &pf.workspaces {
            tags.push(tag(&["workspace", workspace])?);
        }
    }
    Ok(EventBuilder::new(kind(KIND_PROFILE), content)
        .tags(tags)
        .allow_self_tagging())
}

pub(super) fn decode(event: &Event, pubkey: String) -> Option<DomainEvent> {
    let host = first_tag(event, "host").unwrap_or_default().to_string();
    let is_backend = has_bare_tag(event, "backend");
    let name = name_from_metadata(&event.content);
    let agent_slug = agent_slug(event);
    let slug = if is_backend {
        name
    } else {
        crate::idref::session_handle_from_profile_name(&name, &agent_slug)
    };
    Some(DomainEvent::Profile(Profile {
        agent: AgentRef::new(pubkey, slug),
        agent_slug,
        host,
        workspace: if is_backend {
            String::new()
        } else {
            first_tag(event, "workspace")
                .unwrap_or_default()
                .to_string()
        },
        owners: all_tag_values(event, "p"),
        is_backend,
        agents: if is_backend {
            managed_agents(event)
        } else {
            Vec::new()
        },
        workspaces: if is_backend {
            all_tag_values(event, "workspace")
        } else {
            Vec::new()
        },
    }))
}

/// Parse the backend's advertised `["agent", slug, desc]` tags into
/// `(slug, description)`. A missing description defaults to empty (the inventory
/// omits the third element when the agent has no use-criteria byline).
fn managed_agents(event: &Event) -> Vec<(String, String)> {
    event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) != Some("agent") {
                return None;
            }
            let slug = s.get(1)?.clone();
            let desc = s.get(2).cloned().unwrap_or_default();
            Some((slug, desc))
        })
        .collect()
}

fn agent_slug(event: &Event) -> String {
    first_tag(event, "agent-slug")
        .unwrap_or_default()
        .to_string()
}

fn name_from_metadata(content: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| {
            ["display_name", "name"].into_iter().find_map(|key| {
                value
                    .get(key)
                    .and_then(|name| name.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(String::from)
            })
        })
        .unwrap_or_default()
}
