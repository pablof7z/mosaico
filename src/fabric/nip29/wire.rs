//! NIP-29 wire shape for tenex-edge domain events.
//!
//! | Domain      | Wire |
//! |-------------|------|
//! | Profile     | kind:0,     content `{"name": "slug@host"}`, `["host", host]` |
//! | Activity    | kind:1,     `["h", channel]` — social narrative (no inbox routing) |
//! | Status      | kind:30315, content = live activity (may be empty when idle), `["d", session_id]`, one or more `["h", channel]`, `["title", title]` (always), `["status", "busy"\|"idle"]`, `["host", host]`, optional `["slug", slug]`, optional `["rel-cwd", rel]`, optional NIP-40 `["expiration", ts]` |
//! | AgentRoster | kind:30555, backend management-key signed, `["d", capability_slug]`, `["hostname", host]`, `["use-criteria", text]`, one or more root-channel `["h", channel]` |
//! | Chat        | kind:9,     `["h", channel]`, optional `["p", mentioned_pubkey]` |
//!
//! Status is the single self-contained per-session signal: ONE kind:30315 event
//! per `(author_pubkey, session_id)` carries the whole live state (busy/idle, the
//! live activity in the content, the persistent title, host, rel-cwd). It targets
//! every channel the session is in with repeated `h` tags. The optional `slug`
//! tag is a render hint only; the event signer remains the identity authority.
//! Liveness IS the freshness of this event: the daemon re-arms a NIP-40 `["expiration", now +
//! STATUS_TTL_SECS]` tag on every heartbeat, so a stopped session's event ages
//! off the relay shortly after its last beat. A `Status` with `expires_at ==
//! None` publishes no expiration (tests / non-heartbeat contexts). There is no
//! separate presence heartbeat.
//!
//! Chat (kind:9) is the sole agent-to-agent messaging mechanism. Direct messaging
//! uses an inline `@<agent-instance-label>` in the chat body, which adds a `p`
//! tag for the mentioned instance pubkey.
//!
//! Most events resolve slug downstream; status carries an optional render-hint slug. Authorization
//! uses only event.pubkey (signer); self-asserted `agent` tags have no authority
//! and are never written or read.

use crate::domain::{Activity, AgentRef, ChatMessage, DomainEvent, Proposal, Status};
use crate::fabric::{NostrEventCodec, RawEnvelope};
use crate::util::SessionId;
use anyhow::Result;
use nostr_sdk::prelude::*;

pub const KIND_PROFILE: u16 = 0;
pub const KIND_CHAT: u16 = 9;
/// NIP-25 reaction. Used by the daemon to acknowledge a kind:9 routed to a local
/// agent: a 👁 reaction with the channel `h` and `e` (routed event id) tags,
/// signed by the backend management key.
pub const KIND_REACTION: u16 = 7;
pub const KIND_STATUS: u16 = 30315;
pub const KIND_AGENT_ROSTER: u16 = 30555;

// NIP-29 group management (tenexPrivateKey-signed) + relay-authored state.
pub const KIND_GROUP_CREATE: u16 = 9007;
pub const KIND_GROUP_PUT_USER: u16 = 9000;
pub const KIND_GROUP_REMOVE_USER: u16 = 9001;
pub const KIND_GROUP_EDIT_METADATA: u16 = 9002;
pub const KIND_GROUP_METADATA: u16 = 39000;
pub const KIND_GROUP_ADMINS: u16 = 39001;
pub const KIND_GROUP_MEMBERS: u16 = 39002;

// NIP-23 long-form article — used for agent-authored proposals.
pub const KIND_LONGFORM: u16 = 30023;

mod profile;

pub struct Nip29WireCodec;

pub(crate) fn kind(n: u16) -> Kind {
    Kind::from(n)
}

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

pub(crate) fn h_filter(f: Filter, channel: &str) -> Filter {
    f.custom_tag(SingleLetterTag::lowercase(Alphabet::H), channel)
}

fn h_tag(channel: &str) -> Result<Tag> {
    tag(&["h", channel])
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

/// True if any tag has `name` as its sole element (no value — a bare marker tag).
fn has_bare_tag(event: &Event, name: &str) -> bool {
    event.tags.iter().any(|t| {
        let s = t.as_slice();
        s.first().map(String::as_str) == Some(name)
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

fn channel_from_tags(event: &Event) -> Option<String> {
    first_tag(event, "h").map(String::from)
}

impl Nip29WireCodec {
    pub fn encode_event(&self, ev: &DomainEvent) -> Result<EventBuilder> {
        let b = match ev {
            DomainEvent::Profile(pf) => profile::encode(pf)?,
            DomainEvent::Activity(Activity {
                agent: _agent,
                channel,
                text,
            }) => {
                // Activity is a social narrative note (kind:1 without inbox routing).
                // We still encode it for broadcast purposes but it's not part of
                // the inbox system. For now, encode as a plain kind:1 note.
                use nostr_sdk::prelude::EventBuilder as EB;
                EB::new(Kind::from(1u16), text.clone()).tags([h_tag(channel)?])
            }
            DomainEvent::Status(Status {
                agent,
                channels,
                session_id,
                host,
                title,
                activity,
                busy,
                rel_cwd,
                expires_at,
            }) => {
                // The self-contained per-session signal. The replaceable address is
                // `(author_pubkey, d=session_id)`; repeated h tags make the same
                // status visible in every channel the session occupies.
                let mut tags = vec![
                    tag(&["d", session_id.as_str()])?,
                    tag(&["title", title])?,
                    tag(&["status", if *busy { "busy" } else { "idle" }])?,
                    tag(&["host", host])?,
                ];
                for channel in channels {
                    tags.push(h_tag(channel)?);
                }
                // Carry the agent slug on the wire as a convenience hint. The
                // durable agent key IS the author, so peers can resolve it via
                // kind:0; the slug tag avoids that extra round-trip lookup and
                // lets `who` render the name immediately on receipt.
                if !agent.slug.is_empty() {
                    tags.push(tag(&["slug", &agent.slug])?);
                }
                if !rel_cwd.is_empty() {
                    tags.push(tag(&["rel-cwd", rel_cwd])?);
                }
                if let Some(exp) = expires_at {
                    tags.push(tag(&["expiration", &exp.to_string()])?);
                }
                EventBuilder::new(kind(KIND_STATUS), activity.clone()).tags(tags)
            }
            DomainEvent::ChatMessage(ChatMessage {
                from: _from,
                channel,
                body,
                mentioned_pubkey,
            }) => {
                let mut tags = vec![h_tag(channel)?];
                if let Some(pk) = mentioned_pubkey {
                    tags.push(tag(&["p", pk])?);
                }
                EventBuilder::new(kind(KIND_CHAT), body.clone())
                    .tags(tags)
                    .allow_self_tagging()
            }
            DomainEvent::Proposal(Proposal {
                agent: _,
                channel,
                title,
                body,
                d,
                audience,
            }) => {
                let mut tags = vec![
                    tag(&["d", d])?,
                    tag(&["title", title])?,
                    h_tag(channel)?,
                    // No agent tag: author identity is the event signer; slug is in kind:0.
                ];
                // p-tag each owner so the proposal surfaces to the human.
                for owner in audience {
                    tags.push(tag(&["p", owner])?);
                }
                EventBuilder::new(kind(KIND_LONGFORM), body.clone()).tags(tags)
            }
        };
        Ok(b)
    }

    pub fn decode_event(&self, event: &Event) -> Option<DomainEvent> {
        let pubkey = event.pubkey.to_hex();
        match event.kind.as_u16() {
            KIND_PROFILE => profile::decode(event, pubkey),
            KIND_STATUS => {
                let d = first_tag(event, "d")?;
                let channels = all_tag_values(event, "h");
                if channels.is_empty() {
                    return None;
                }
                Some(DomainEvent::Status(Status {
                    // Slug rides as a convenience tag (avoids a kind:0 lookup);
                    // empty on legacy emitters, resolved downstream from kind:0.
                    agent: AgentRef::new(
                        pubkey,
                        first_tag(event, "slug").unwrap_or_default().to_string(),
                    ),
                    channels,
                    session_id: SessionId::from(d),
                    host: first_tag(event, "host").unwrap_or_default().to_string(),
                    title: first_tag(event, "title").unwrap_or_default().to_string(),
                    // The live activity is the event content (empty when idle).
                    activity: event.content.clone(),
                    busy: first_tag(event, "status") == Some("busy"),
                    rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                    // NIP-40 expiration → liveness clock. Absent → None.
                    expires_at: first_tag(event, "expiration").and_then(|s| s.parse().ok()),
                }))
            }
            KIND_CHAT => Some(DomainEvent::ChatMessage(ChatMessage {
                // Slug is NOT on the wire; resolved by the materializer.
                from: AgentRef::new(pubkey, String::new()),
                channel: channel_from_tags(event)?,
                body: event.content.clone(),
                mentioned_pubkey: first_tag(event, "p").map(str::to_string),
            })),
            1 => {
                // kind:1 notes: decode as Activity for social narrative (no routing).
                let channel = channel_from_tags(event)?;
                Some(DomainEvent::Activity(Activity {
                    agent: AgentRef::new(pubkey, String::new()),
                    channel,
                    text: event.content.clone(),
                }))
            }
            KIND_LONGFORM => Some(DomainEvent::Proposal(Proposal {
                // Slug is NOT on the wire; resolved downstream from kind:0 profile.
                agent: AgentRef::new(pubkey, String::new()),
                channel: channel_from_tags(event)?,
                title: first_tag(event, "title").unwrap_or_default().to_string(),
                body: event.content.clone(),
                d: first_tag(event, "d").unwrap_or_default().to_string(),
                audience: all_tag_values(event, "p"),
            })),
            _ => None,
        }
    }
}

impl NostrEventCodec for Nip29WireCodec {
    fn encode(&self, ev: &DomainEvent) -> Result<EventBuilder> {
        self.encode_event(ev)
    }

    fn decode(&self, env: &RawEnvelope) -> Option<DomainEvent> {
        match env {
            RawEnvelope::Nostr(event) => self.decode_event(event),
        }
    }
}

#[cfg(test)]
mod tests;
