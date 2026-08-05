//! NIP-29 wire shape for mosaico domain events.
//!
//! | Domain      | Wire |
//! |-------------|------|
//! | Profile     | kind:0,     content `{"name": "sessionCode-agent"}`, `["host", host]`, optional `["agent-slug", slug]` and scoped live-agent `["workspace", root_h]`; backend profiles additionally carry `["backend"]`, repeated `["agent", slug, desc]`, and repeated `["workspace", root_h]` tags |
//! | Status      | kind:30315, content = live activity (may be empty between turns), `["d", "status"]`, one or more `["h", channel]`, `["title", title]` (always), `["state", "working"\|"idle"\|"suspended"\|"offline"]`, `["state-since", ts]`, `["host", host]`, `["workspace", root-name]`, optional `["branch", branch]`, optional `["slug", slug]`, optional `["rel-cwd", rel]`, optional NIP-40 `["expiration", ts]` |
//! | Chat        | kind:9,     `["h", channel]`, repeated `["p", mentioned_pubkey]`, repeated `["attachment", URL, LABEL]` |
//!
//! Status is the single self-contained per-agent signal: ONE kind:30315 event
//! per author pubkey carries the whole canonical live state, the
//! live activity in the content, the persistent title, host, rel-cwd). It targets
//! every channel the session is in with repeated `h` tags. The optional `slug`
//! tag is a render hint only; the event signer remains the identity authority.
//! Remote liveness is the freshness of this lease: the daemon re-arms a NIP-40
//! `["expiration", now + PRESENCE_LEASE_TTL_SECS]` tag on renewal, so a stopped
//! session ages off the relay. A `Status` with no expiry is reserved for tests.
//! There is no second liveness signal.
//!
//! Chat (kind:9) is the sole agent-to-agent messaging mechanism. Direct messaging
//! uses an inline `@<agent-instance-label>` in the chat body, which adds a `p`
//! tag for the mentioned instance pubkey.
//!
//! Most events resolve slug downstream; status carries an optional render-hint slug. Authorization
//! uses only event.pubkey (signer). Self-asserted `agent` tags on *agent-session* kind:0s have no
//! authority and are never written; only the **backend** management-key-signed kind:0 advertises
//! `["agent", slug, desc]` tags (the host inventory for client add-agent pickers).

use crate::domain::{AgentRef, ChatAttachment, ChatMessage, DomainEvent, Reaction, Status};
use crate::fabric::{NostrEventCodec, RawEnvelope};
use anyhow::Result;
use nostr::*;

pub const KIND_PROFILE: u16 = 0;
pub const KIND_CHAT: u16 = 9;
/// NIP-25 reaction. Used by the daemon to acknowledge a kind:9 routed to a local
/// agent: a 👁 reaction with the channel `h` and `e` (routed event id) tags,
/// signed by the backend management key.
pub const KIND_REACTION: u16 = 7;
pub const KIND_STATUS: u16 = 30315;

// NIP-29 group management (mosaicoPrivateKey-signed) + relay-authored state.
pub const KIND_GROUP_CREATE: u16 = 9007;
pub const KIND_GROUP_DELETE: u16 = 9008;
pub const KIND_GROUP_PUT_USER: u16 = 9000;
pub const KIND_GROUP_REMOVE_USER: u16 = 9001;
pub const KIND_GROUP_EDIT_METADATA: u16 = 9002;
pub const KIND_GROUP_METADATA: u16 = 39000;
pub const KIND_GROUP_ADMINS: u16 = 39001;
pub const KIND_GROUP_MEMBERS: u16 = 39002;

mod profile;

pub struct Nip29WireCodec;

pub(crate) fn kind(n: u16) -> Kind {
    Kind::from(n)
}

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
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

/// `["attachment", url, label, sha256]`. The digest is required, not optional:
/// a receiver cannot verify a blob it has no expected hash for, and an
/// attachment that cannot be verified is one this build will not place on disk.
fn attachment_tags(event: &Event) -> Vec<ChatAttachment> {
    let mut attachments = Vec::new();
    for tag in event.tags.iter() {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("attachment") || values.len() != 4 {
            continue;
        }
        let candidate = ChatAttachment {
            url: values[1].clone(),
            label: values[2].clone(),
            sha256: values[3].clone(),
        };
        crate::attachment_contract::try_push(&mut attachments, candidate);
    }
    attachments
}

fn channel_from_tags(event: &Event) -> Option<String> {
    first_tag(event, "h").map(String::from)
}

impl Nip29WireCodec {
    pub fn encode_event(&self, ev: &DomainEvent) -> Result<EventBuilder> {
        let b = match ev {
            DomainEvent::Profile(pf) => profile::encode(pf)?,
            DomainEvent::Status(Status {
                agent,
                channels,
                host,
                workspace,
                branch,
                title,
                activity,
                state,
                state_since,
                rel_cwd,
                expires_at,
                dispatch_event,
            }) => {
                // The self-contained per-agent signal. The replaceable address is
                // `(author_pubkey, d=status)`; repeated h tags make the same
                // status visible in every channel the session occupies.
                let mut tags = vec![
                    tag(&["d", "status"])?,
                    tag(&["title", title])?,
                    tag(&["state", state.as_str()])?,
                    tag(&["state-since", &state_since.to_string()])?,
                    tag(&["host", host])?,
                    tag(&["workspace", workspace])?,
                    tag(&["slug", &agent.slug])?,
                ];
                // The one draft in Mosaico that still writes its own context
                // rows, because it needs SEVERAL and no NMP door mints more
                // than one: `nip29::Group` is one scope plus one group id.
                // See `NmpHost::enqueue_multi_group_event` and
                // pablof7z/nmp#1281.
                for channel in channels {
                    tags.push(h_tag(channel)?);
                }
                if !rel_cwd.is_empty() {
                    tags.push(tag(&["rel-cwd", rel_cwd])?);
                }
                if !branch.is_empty() {
                    tags.push(tag(&["branch", branch])?);
                }
                if let Some(exp) = expires_at {
                    tags.push(tag(&["expiration", &exp.to_string()])?);
                }
                if let Some(dispatch_event) = dispatch_event.as_deref().filter(|s| !s.is_empty()) {
                    tags.push(tag(&["e", dispatch_event])?);
                }
                EventBuilder::new(kind(KIND_STATUS), activity.clone()).tags(tags)
            }
            DomainEvent::ChatMessage(ChatMessage {
                from: _from,
                channel: _channel,
                body,
                mentioned_pubkeys,
                attachments,
            }) => {
                crate::attachment_contract::validate_attachments(attachments)?;
                // No `h` row. `channel` names the group this draft is published
                // INTO, and the context tag is NMP's group door to mint, before
                // the bytes are signed.
                let mut tags = Vec::new();
                for pk in mentioned_pubkeys {
                    tags.push(tag(&["p", pk])?);
                }
                for attachment in attachments {
                    tags.push(tag(&[
                        "attachment",
                        &attachment.url,
                        &attachment.label,
                        &attachment.sha256,
                    ])?);
                }
                EventBuilder::new(kind(KIND_CHAT), body.clone())
                    .tags(tags)
                    .allow_self_tagging()
            }
            DomainEvent::Reaction(Reaction {
                reactor: _reactor,
                channel: _channel,
                target_event_id,
                emoji,
            }) => {
                // NIP-25 reaction: content = emoji, `e` = target message id. The
                // group scoping that gets it admitted and attributed is the `h`
                // row NMP's group door mints; `channel` selects which door.
                let tags = vec![tag(&["e", target_event_id])?];
                EventBuilder::new(kind(KIND_REACTION), emoji.clone()).tags(tags)
            }
        };
        Ok(b)
    }

    pub fn decode_event(&self, event: &Event) -> Option<DomainEvent> {
        let pubkey = event.pubkey.to_hex();
        match event.kind.as_u16() {
            KIND_PROFILE => profile::decode(event, pubkey),
            KIND_STATUS => {
                if first_tag(event, "d")? != "status" {
                    return None;
                }
                let channels = all_tag_values(event, "h");
                Some(DomainEvent::Status(Status {
                    agent: AgentRef::new(pubkey, first_tag(event, "slug")?.to_string()),
                    channels,
                    host: first_tag(event, "host")?.to_string(),
                    workspace: first_tag(event, "workspace")?.to_string(),
                    branch: first_tag(event, "branch").unwrap_or_default().to_string(),
                    title: first_tag(event, "title")?.to_string(),
                    // The live activity is the event content (empty when idle).
                    activity: event.content.clone(),
                    state: crate::session_state::SessionState::parse(first_tag(event, "state")?)?,
                    state_since: first_tag(event, "state-since")?.parse().ok()?,
                    rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                    // NIP-40 expiration → liveness clock. Absent → None.
                    expires_at: first_tag(event, "expiration").and_then(|s| s.parse().ok()),
                    dispatch_event: first_tag(event, "e").map(str::to_string),
                }))
            }
            KIND_CHAT => Some(DomainEvent::ChatMessage(ChatMessage {
                // Slug is NOT on the wire; resolved by the materializer.
                from: AgentRef::new(pubkey, String::new()),
                channel: channel_from_tags(event)?,
                body: event.content.clone(),
                mentioned_pubkeys: all_tag_values(event, "p"),
                attachments: attachment_tags(event),
            })),
            KIND_REACTION => {
                // A reaction MUST reference a target message via an `e` tag. A
                // bare kind:7 (no `e`) is not a domain reaction — returning None
                // lets it fall through to the verbatim relay_events cache.
                let target_event_id = first_tag(event, "e")?.to_string();
                // TRUST BOUNDARY: the content of an inbound kind:7 is untrusted —
                // an adversarial member could e-tag one of the target's messages
                // with a large or multi-line natural-language payload that would
                // otherwise land verbatim in the target's turn-start awareness
                // (prompt injection / token bloat). Reject anything that is not a
                // bounded emoji here; an invalid reaction falls through to the
                // verbatim relay_events cache and is never surfaced as awareness.
                if !Reaction::emoji_is_valid(&event.content) {
                    return None;
                }
                Some(DomainEvent::Reaction(Reaction {
                    // Slug is NOT on the wire; resolved downstream from kind:0.
                    reactor: AgentRef::new(pubkey, String::new()),
                    channel: channel_from_tags(event).unwrap_or_default(),
                    target_event_id,
                    emoji: event.content.clone(),
                }))
            }
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
