//! The `kind1` codec set — tenex-edge's initial wire shape (M1 §3).
//!
//! | Domain    | Wire |
//! |-----------|------|
//! | Profile   | kind:0,    content `{"name": slug}`, `["host", host]` |
//! | Presence  | kind:30315 (NIP-38-style heartbeat), `["h", project]`, `["d", "tenex-edge-presence:<session>"]`, `["p", peer]…`, `["agent", pk, slug]`, `["session-id", id]`, `["host", host]`, optional `["rel-cwd", rel]`, `["expiration", ts]` |
//! | Activity   | kind:1,    `["h", project]` |
//! | TurnReply  | kind:1,    `["h", project]`, `["e", root_id, "", "root"]`, `["e", reply_id, "", "reply"]` |
//! | Status     | kind:30315 (NIP-38), `["h", project]`, `["d", project]`, `["agent", pk, slug]`, optional `["session-id", id]`, optional `["rel-cwd", rel]`, `["expiration", ts]` |
//! | Mention    | kind:1,    `["h", project]`, `["p", to]`, optional `["session-id", target]`, optional `["from-session", sender]` |
//!
//! kind:1 disambiguation on decode (in priority order):
//!   1. Has `["p", ...]` tag                   → Mention
//!   2. Has `["e", ..., "", "root"]` NIP-10 tag → TurnReply
//!   3. Otherwise                               → Activity
//!
//! Sender identity is the event `pubkey`; slug is resolved from the profile store
//! at routing time and is NOT carried on the wire for kind:1 events.
//!
//! For kind:30023 long-form articles generated during a session, the same
//! `root_event_id` from the session's TurnReply thread should be e-tagged so the
//! article can be linked back to the conversation that produced it.

use crate::codec::{Codec, SubScope};
use crate::domain::{
    Activity, AgentRef, DomainEvent, Mention, Presence, Profile, Status, TurnReply,
};
use crate::util::SessionId;
use anyhow::Result;
use nostr_sdk::prelude::*;

pub const KIND_PROFILE: u16 = 0;
pub const KIND_PRESENCE: u16 = 30315;
pub const KIND_NOTE: u16 = 1;
pub const KIND_STATUS: u16 = 30315;

const PRESENCE_D_PREFIX: &str = "tenex-edge-presence:";

mod filters;
mod groups;

pub use groups::{
    group_create, group_lock_closed, group_put_user, KIND_GROUP_ADMINS, KIND_GROUP_CREATE,
    KIND_GROUP_EDIT_METADATA, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA, KIND_GROUP_PUT_USER,
};

pub struct Kind1Codec;

fn kind(n: u16) -> Kind {
    Kind::from(n)
}

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

fn h_filter(f: Filter, project: &str) -> Filter {
    f.custom_tag(SingleLetterTag::lowercase(Alphabet::H), project)
}

fn project_tag(project: &str) -> Result<Tag> {
    tag(&["h", project])
}

fn presence_d(session_id: &str) -> String {
    format!("{PRESENCE_D_PREFIX}{session_id}")
}

/// First value of the first tag whose name matches `name` (i.e. `slice[1]`).
fn first_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

/// All values (`slice[1]`) of every tag named `name`.
fn all_tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some(name) {
                s.get(1).cloned()
            } else {
                None
            }
        })
        .collect()
}

fn project_from_tags(event: &Event) -> Option<String> {
    first_tag(event, "h").map(String::from)
}

/// Slug from an `["agent", pubkey, slug]` tag (`slice[2]`).
fn agent_slug(event: &Event) -> String {
    event
        .tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some("agent") {
                s.get(2).cloned()
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// Value (`slice[1]`) of the first `["e", id, relay, marker]` tag whose marker
/// (`slice[3]`) matches. Used to extract NIP-10 root/reply references.
fn e_tag_with_marker<'a>(event: &'a Event, marker: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some("e")
            && s.get(3).map(String::as_str) == Some(marker)
        {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

fn name_from_metadata(content: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        .unwrap_or_default()
}

impl Codec for Kind1Codec {
    fn name(&self) -> &'static str {
        "kind1"
    }

    fn encode(&self, ev: &DomainEvent) -> Result<EventBuilder> {
        let b = match ev {
            DomainEvent::Profile(Profile {
                agent,
                host,
                owners,
            }) => {
                let content = serde_json::json!({ "name": agent.slug }).to_string();
                let mut tags = vec![tag(&["host", host])?];
                for o in owners {
                    tags.push(tag(&["p", o])?); // declare the human owner(s)
                }
                EventBuilder::new(kind(KIND_PROFILE), content)
                    .tags(tags)
                    .allow_self_tagging()
            }
            DomainEvent::Presence(Presence {
                agent,
                project,
                session_id,
                host,
                rel_cwd,
                audience,
                expires_at,
            }) => {
                let mut tags = Vec::new();
                for p in audience {
                    tags.push(tag(&["p", p])?);
                }
                let d = presence_d(session_id.as_str());
                tags.push(project_tag(project)?);
                tags.push(tag(&["d", &d])?);
                tags.push(tag(&["agent", &agent.pubkey, &agent.slug])?);
                tags.push(tag(&["session-id", session_id.as_str()])?);
                tags.push(tag(&["host", host])?);
                if !rel_cwd.is_empty() {
                    tags.push(tag(&["rel-cwd", rel_cwd])?);
                }
                tags.push(tag(&["expiration", &expires_at.to_string()])?);
                EventBuilder::new(kind(KIND_PRESENCE), "online")
                    .tags(tags)
                    .allow_self_tagging()
            }
            DomainEvent::Activity(Activity {
                agent: _,
                project,
                text,
            }) => EventBuilder::new(kind(KIND_NOTE), text.clone()).tags([project_tag(project)?]),
            DomainEvent::Status(Status {
                agent,
                project,
                session_id,
                text,
                rel_cwd,
                expires_at,
            }) => {
                let mut tags = vec![
                    project_tag(project)?,
                    tag(&["d", project])?,
                    tag(&["agent", &agent.pubkey, &agent.slug])?,
                ];
                if let Some(session_id) = session_id {
                    tags.push(tag(&["session-id", session_id.as_str()])?);
                }
                if !rel_cwd.is_empty() {
                    tags.push(tag(&["rel-cwd", rel_cwd])?);
                }
                if let Some(exp) = expires_at {
                    tags.push(tag(&["expiration", &exp.to_string()])?);
                }
                EventBuilder::new(kind(KIND_STATUS), text.clone()).tags(tags)
            }
            DomainEvent::Mention(Mention {
                from: _,
                to_pubkey,
                project,
                body,
                target_session,
                from_session,
            }) => {
                let mut tags = vec![project_tag(project)?, tag(&["p", to_pubkey])?];
                if let Some(sess) = target_session {
                    tags.push(tag(&["session-id", sess.as_str()])?);
                }
                if let Some(sess) = from_session {
                    tags.push(tag(&["from-session", sess.as_str()])?);
                }
                // allow_self_tagging: a mention to a sibling session of the SAME
                // agent has p == author; nostr would otherwise strip that p tag.
                EventBuilder::new(kind(KIND_NOTE), body.clone())
                    .tags(tags)
                    .allow_self_tagging()
            }
            DomainEvent::TurnReply(TurnReply {
                agent: _,
                project,
                body,
                root_event_id,
                reply_event_id,
            }) => EventBuilder::new(kind(KIND_NOTE), body.clone()).tags([
                project_tag(project)?,
                tag(&["e", root_event_id, "", "root"])?,
                tag(&["e", reply_event_id, "", "reply"])?,
            ]),
        };
        Ok(b)
    }

    fn decode(&self, event: &Event) -> Option<DomainEvent> {
        let pubkey = event.pubkey.to_hex();
        match event.kind.as_u16() {
            KIND_PROFILE => Some(DomainEvent::Profile(Profile {
                agent: AgentRef::new(pubkey, name_from_metadata(&event.content)),
                host: first_tag(event, "host").unwrap_or_default().to_string(),
                owners: all_tag_values(event, "p"),
            })),
            KIND_STATUS => {
                let expires_at = first_tag(event, "expiration").and_then(|s| s.parse().ok());
                let d = first_tag(event, "d").unwrap_or_default();
                if d.starts_with(PRESENCE_D_PREFIX) {
                    let session_id = first_tag(event, "session-id")?;
                    Some(DomainEvent::Presence(Presence {
                        agent: AgentRef::new(pubkey, agent_slug(event)),
                        project: project_from_tags(event)?,
                        session_id: SessionId::from(session_id),
                        host: first_tag(event, "host").unwrap_or_default().to_string(),
                        rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                        audience: all_tag_values(event, "p"),
                        expires_at: expires_at?,
                    }))
                } else {
                    Some(DomainEvent::Status(Status {
                        agent: AgentRef::new(pubkey, agent_slug(event)),
                        project: project_from_tags(event)?,
                        session_id: first_tag(event, "session-id").map(SessionId::from),
                        text: event.content.clone(),
                        rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                        expires_at,
                    }))
                }
            }
            KIND_NOTE => {
                let project = project_from_tags(event)?;
                // Disambiguation (in priority order):
                //   1. Has p tag                            → Mention
                //   2. Has NIP-10 e-tag with "root" marker  → TurnReply
                //   3. Otherwise                            → Activity
                if let Some(to) = first_tag(event, "p") {
                    return Some(DomainEvent::Mention(Mention {
                        from: AgentRef::new(pubkey, ""),
                        to_pubkey: to.to_string(),
                        project,
                        body: event.content.clone(),
                        target_session: first_tag(event, "session-id").map(SessionId::from),
                        from_session: first_tag(event, "from-session").map(SessionId::from),
                    }));
                }
                if let (Some(root_id), Some(reply_id)) = (
                    e_tag_with_marker(event, "root"),
                    e_tag_with_marker(event, "reply"),
                ) {
                    return Some(DomainEvent::TurnReply(TurnReply {
                        agent: AgentRef::new(pubkey, ""),
                        project,
                        body: event.content.clone(),
                        root_event_id: root_id.to_string(),
                        reply_event_id: reply_id.to_string(),
                    }));
                }
                Some(DomainEvent::Activity(Activity {
                    agent: AgentRef::new(pubkey, ""),
                    project,
                    text: event.content.clone(),
                }))
            }
            _ => None,
        }
    }

    fn filters(&self, scope: &SubScope) -> Vec<Filter> {
        filters::filters(scope)
    }
}

#[cfg(test)]
mod tests;
