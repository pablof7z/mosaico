//! The `kind1` codec set — tenex-edge's initial wire shape (M1 §3).
//!
//! | Domain    | Wire |
//! |-----------|------|
//! | Profile   | kind:0,    content `{"name": slug}`, `["host", host]` |
//! | Presence  | kind:30315 (NIP-38-style heartbeat), `["h", project]`, `["d", "tenex-edge-presence:<session>"]`, `["p", peer]…`, `["agent", pk, slug]`, `["session-id", id]`, `["host", host]`, optional `["rel-cwd", rel]`, `["expiration", ts]` |
//! | Activity  | kind:1,    `["h", project]`, `["agent", pk, slug]` |
//! | Status    | kind:30315 (NIP-38), `["h", project]`, `["d", project]`, `["agent", pk, slug]`, optional `["rel-cwd", rel]`, `["expiration", ts]` |
//! | Mention   | kind:1,    `["h", project]`, `["p", to]`, `["agent", pk, slug]`, optional `["session-id", target]` |
//!
//! Activity vs Mention (both kind:1) is disambiguated on decode by the presence
//! of a `p` tag.

use crate::codec::{Codec, SubScope};
use crate::domain::{Activity, AgentRef, DomainEvent, Mention, Presence, Profile, Status};
use anyhow::Result;
use nostr_sdk::prelude::*;

pub const KIND_PROFILE: u16 = 0;
pub const KIND_PRESENCE: u16 = 30315;
pub const KIND_NOTE: u16 = 1;
pub const KIND_STATUS: u16 = 30315;

// NIP-29 group management (operator/userNsec-signed) + relay-authored state.
pub const KIND_GROUP_CREATE: u16 = 9007;
pub const KIND_GROUP_PUT_USER: u16 = 9000;
pub const KIND_GROUP_EDIT_METADATA: u16 = 9002;
pub const KIND_GROUP_METADATA: u16 = 39000;
pub const KIND_GROUP_ADMINS: u16 = 39001;
pub const KIND_GROUP_MEMBERS: u16 = 39002;

const PRESENCE_D_PREFIX: &str = "tenex-edge-presence:";

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

// ── NIP-29 group management builders (signed by the operator's userNsec) ──────
//
// These sit outside the DomainEvent flow: they manage the relay's group, they
// aren't fabric domain events. The relay rules these encode were validated by
// `tests/nip29_probe.rs` against nip29.f7z.io. Recipe for an owned closed group:
//   group_create -> group_lock_closed -> group_put_user (per agent).

/// kind:9007 create-group with a client-chosen id (`h` == project slug). The
/// signer becomes the group admin. NOTE: a fresh group is OPEN until locked.
pub fn group_create(project: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_CREATE), "").tags([project_tag(project)?]))
}

/// kind:9002 edit-metadata that locks the group `closed` (only members may write)
/// while keeping it `public` (anyone may read — required so the non-member daemon
/// connection still receives group events). Names the group after the slug.
pub fn group_lock_closed(project: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_EDIT_METADATA), "").tags([
        project_tag(project)?,
        tag(&["name", project])?,
        tag(&["closed"])?,
        tag(&["public"])?,
    ]))
}

/// kind:9000 put-user adding `pubkey` to the group as a member, so it can publish
/// presence/activity/mentions into the now-closed group.
pub fn group_put_user(project: &str, pubkey: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_PUT_USER), "").tags([
        project_tag(project)?,
        tag(&["p", pubkey, "member"])?,
    ]))
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
                let d = presence_d(session_id);
                tags.push(project_tag(project)?);
                tags.push(tag(&["d", &d])?);
                tags.push(tag(&["agent", &agent.pubkey, &agent.slug])?);
                tags.push(tag(&["session-id", session_id])?);
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
                agent,
                project,
                text,
            }) => EventBuilder::new(kind(KIND_NOTE), text.clone()).tags([
                project_tag(project)?,
                tag(&["agent", &agent.pubkey, &agent.slug])?,
            ]),
            DomainEvent::Status(Status {
                agent,
                project,
                text,
                rel_cwd,
                expires_at,
            }) => {
                let mut tags = vec![
                    project_tag(project)?,
                    tag(&["d", project])?,
                    tag(&["agent", &agent.pubkey, &agent.slug])?,
                ];
                if !rel_cwd.is_empty() {
                    tags.push(tag(&["rel-cwd", rel_cwd])?);
                }
                if let Some(exp) = expires_at {
                    tags.push(tag(&["expiration", &exp.to_string()])?);
                }
                EventBuilder::new(kind(KIND_STATUS), text.clone()).tags(tags)
            }
            DomainEvent::Mention(Mention {
                from,
                to_pubkey,
                project,
                body,
                target_session,
            }) => {
                let mut tags = vec![
                    project_tag(project)?,
                    tag(&["p", to_pubkey])?,
                    tag(&["agent", &from.pubkey, &from.slug])?,
                ];
                if let Some(sess) = target_session {
                    tags.push(tag(&["session-id", sess])?);
                }
                // allow_self_tagging: a mention to a sibling session of the SAME
                // agent has p == author; nostr would otherwise strip that p tag.
                EventBuilder::new(kind(KIND_NOTE), body.clone())
                    .tags(tags)
                    .allow_self_tagging()
            }
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
                if let Some(session_id) = first_tag(event, "session-id") {
                    Some(DomainEvent::Presence(Presence {
                        agent: AgentRef::new(pubkey, agent_slug(event)),
                        project: project_from_tags(event)?,
                        session_id: session_id.to_string(),
                        host: first_tag(event, "host").unwrap_or_default().to_string(),
                        rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                        audience: all_tag_values(event, "p"),
                        expires_at: expires_at?,
                    }))
                } else {
                    Some(DomainEvent::Status(Status {
                        agent: AgentRef::new(pubkey, agent_slug(event)),
                        project: project_from_tags(event)?,
                        text: event.content.clone(),
                        rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                        expires_at,
                    }))
                }
            }
            KIND_NOTE => {
                let project = project_from_tags(event)?;
                let agent = AgentRef::new(pubkey, agent_slug(event));
                // Require an `agent` tag to classify as Mention. User-posted OPs
                // carry a `p` tag (targeting the agent) but no `agent` tag, so
                // they must not be re-injected as intra-agent messages.
                let has_agent = first_tag(event, "agent").is_some();
                if has_agent {
                    if let Some(to) = first_tag(event, "p") {
                        return Some(DomainEvent::Mention(Mention {
                            from: agent,
                            to_pubkey: to.to_string(),
                            project,
                            body: event.content.clone(),
                            target_session: first_tag(event, "session-id").map(String::from),
                        }));
                    }
                }
                Some(DomainEvent::Activity(Activity {
                    agent,
                    project,
                    text: event.content.clone(),
                }))
            }
            _ => None,
        }
    }

    fn filters(&self, scope: &SubScope) -> Vec<Filter> {
        let authors: Vec<PublicKey> = scope
            .authors
            .iter()
            .filter_map(|h| PublicKey::from_hex(h).ok())
            .collect();

        let with_authors = |mut f: Filter| -> Filter {
            if !authors.is_empty() {
                f = f.authors(authors.clone());
            }
            f
        };

        let mut filters = Vec::new();

        // Profiles (kind:0) — identity resolution.
        filters.push(with_authors(Filter::new().kind(kind(KIND_PROFILE))));

        // Presence + status (kind:30315) — live sessions and current work.
        let mut presence_status = Filter::new().kind(kind(KIND_STATUS));
        if let Some(p) = &scope.project {
            presence_status = h_filter(presence_status, p);
        }
        // Group-scoped events are not author-gated locally; the relay enforces
        // membership for groups this daemon owns (created closed via userNsec).
        filters.push(presence_status);

        // Notes (kind:1) — activity + mentions.
        let mut notes = Filter::new().kind(kind(KIND_NOTE));
        if let Some(p) = &scope.project {
            notes = h_filter(notes, p);
        }
        filters.push(notes);

        // Mentions addressed to me (may arrive without a project group match).
        if let Some(me) = &scope.mentions_to {
            if let Ok(pk) = PublicKey::from_hex(me) {
                filters.push(Filter::new().kind(kind(KIND_NOTE)).pubkey(pk));
            }
        }

        // Discover ANY profile (any author) that claims one of our owners — the
        // ACL pending set. Deliberately NOT author-restricted.
        for owner in &scope.owners {
            if let Ok(pk) = PublicKey::from_hex(owner) {
                filters.push(Filter::new().kind(kind(KIND_PROFILE)).pubkey(pk));
            }
        }

        // NIP-29 relay-authored group state (metadata/admins/members) for the
        // scoped group. Keeping this live is "check which groups we own at all
        // times": it feeds the membership cache. Addressable + relay-signed, so
        // filter by the `d` tag (group id == project slug), never by author.
        if let Some(p) = &scope.project {
            filters.push(
                Filter::new()
                    .kinds([
                        kind(KIND_GROUP_METADATA),
                        kind(KIND_GROUP_ADMINS),
                        kind(KIND_GROUP_MEMBERS),
                    ])
                    .identifier(p),
            );
        }

        filters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(ev: DomainEvent, keys: &Keys) -> DomainEvent {
        let codec = Kind1Codec;
        let builder = codec.encode(&ev).expect("encode");
        let signed = builder.sign_with_keys(keys).expect("sign");
        codec.decode(&signed).expect("decode")
    }

    fn agent(keys: &Keys, slug: &str) -> AgentRef {
        AgentRef::new(keys.public_key().to_hex(), slug)
    }

    fn has_tag(event: &Event, name: &str, value: &str) -> bool {
        event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some(name)
                && s.get(1).map(String::as_str) == Some(value)
        })
    }

    fn has_tag_name(event: &Event, name: &str) -> bool {
        event
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(String::as_str) == Some(name))
    }

    #[test]
    fn profile_roundtrip() {
        let keys = Keys::generate();
        let ev = DomainEvent::Profile(Profile {
            agent: agent(&keys, "coder"),
            host: "pablos' laptop".into(),
            owners: vec!["09d4".repeat(16)],
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
    }

    #[test]
    fn presence_roundtrip() {
        let keys = Keys::generate();
        let ev = DomainEvent::Presence(Presence {
            agent: agent(&keys, "coder"),
            project: "tenex-edge".into(),
            session_id: "sess-123".into(),
            host: "laptop".into(),
            rel_cwd: String::new(),
            audience: vec!["aa".repeat(32), "bb".repeat(32)],
            expires_at: 1_900_000_000,
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
    }

    #[test]
    fn presence_rel_cwd_roundtrips_and_emits_tag() {
        let keys = Keys::generate();
        let ev = DomainEvent::Presence(Presence {
            agent: agent(&keys, "coder"),
            project: "tenex-edge".into(),
            session_id: "sess-123".into(),
            host: "laptop".into(),
            rel_cwd: "worktree1".into(),
            audience: vec!["aa".repeat(32)],
            expires_at: 1_900_000_000,
        });
        // The relative dir survives encode→decode …
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
        // … and lands as a `rel-cwd` tag on the wire.
        let signed = Kind1Codec.encode(&ev).unwrap().sign_with_keys(&keys).unwrap();
        assert!(has_tag(&signed, "rel-cwd", "worktree1"));
    }

    #[test]
    fn empty_rel_cwd_emits_no_tag_and_decodes_empty() {
        // Wire compat: events without a rel-cwd tag (old peers) decode to "".
        let keys = Keys::generate();
        let ev = DomainEvent::Presence(Presence {
            agent: agent(&keys, "coder"),
            project: "tenex-edge".into(),
            session_id: "sess-1".into(),
            host: "laptop".into(),
            rel_cwd: String::new(),
            audience: vec![],
            expires_at: 1_900_000_000,
        });
        let signed = Kind1Codec.encode(&ev).unwrap().sign_with_keys(&keys).unwrap();
        assert!(!has_tag_name(&signed, "rel-cwd"));
        match Kind1Codec.decode(&signed) {
            Some(DomainEvent::Presence(p)) => assert_eq!(p.rel_cwd, ""),
            other => panic!("expected presence, got {other:?}"),
        }
    }

    #[test]
    fn presence_uses_session_scoped_nip38_heartbeat() {
        let keys = Keys::generate();
        let ev = DomainEvent::Presence(Presence {
            agent: agent(&keys, "coder"),
            project: "tenex-edge".into(),
            session_id: "sess-123".into(),
            host: "laptop".into(),
            rel_cwd: String::new(),
            audience: vec!["aa".repeat(32)],
            expires_at: 1_900_000_000,
        });
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(signed.kind.as_u16(), KIND_PRESENCE);
        assert!(has_tag(&signed, "h", "tenex-edge"));
        assert!(has_tag(&signed, "d", "tenex-edge-presence:sess-123"));
        assert!(has_tag(&signed, "session-id", "sess-123"));
        assert!(has_tag(&signed, "expiration", "1900000000"));
    }

    #[test]
    fn activity_roundtrip() {
        let keys = Keys::generate();
        let ev = DomainEvent::Activity(Activity {
            agent: agent(&keys, "coder"),
            project: "tenex-edge".into(),
            text: "fixing the auth bug".into(),
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
    }

    #[test]
    fn activity_uses_nip29_h_tag_not_hashtag() {
        let keys = Keys::generate();
        let ev = DomainEvent::Activity(Activity {
            agent: agent(&keys, "coder"),
            project: "tenex-edge".into(),
            text: "fixing the auth bug".into(),
        });
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(has_tag(&signed, "h", "tenex-edge"));
        assert!(!has_tag_name(&signed, "t"));
    }

    #[test]
    fn status_roundtrip_with_expiry() {
        let keys = Keys::generate();
        let ev = DomainEvent::Status(Status {
            agent: agent(&keys, "coder"),
            project: "tenex-edge".into(),
            text: "reviewing PR".into(),
            rel_cwd: String::new(),
            expires_at: Some(1_900_000_000),
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
    }

    #[test]
    fn mention_roundtrip_session_targeted() {
        let keys = Keys::generate();
        let ev = DomainEvent::Mention(Mention {
            from: agent(&keys, "coder"),
            to_pubkey: "cc".repeat(32),
            project: "tenex-edge".into(),
            body: "can you review?".into(),
            target_session: Some("sess-xyz".into()),
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
    }

    #[test]
    fn mention_uses_nip29_h_tag_not_hashtag() {
        let keys = Keys::generate();
        let ev = DomainEvent::Mention(Mention {
            from: agent(&keys, "coder"),
            to_pubkey: "cc".repeat(32),
            project: "tenex-edge".into(),
            body: "can you review?".into(),
            target_session: Some("sess-xyz".into()),
        });
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(has_tag(&signed, "h", "tenex-edge"));
        assert!(!has_tag_name(&signed, "t"));
    }

    #[test]
    fn mention_to_self_keeps_p_tag() {
        // A mention from one session of an agent to another session of the SAME
        // agent has to_pubkey == the signer's own pubkey. Ensure the p tag survives.
        let keys = Keys::generate();
        let pk = keys.public_key().to_hex();
        let ev = DomainEvent::Mention(Mention {
            from: AgentRef::new(pk.clone(), "claude"),
            to_pubkey: pk.clone(),
            project: "p".into(),
            body: "hi".into(),
            target_session: Some("s2".into()),
        });
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        let has_p = signed
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("p"));
        assert!(
            has_p,
            "p tag missing! tags={:?}",
            signed
                .tags
                .iter()
                .map(|t| t.as_slice().to_vec())
                .collect::<Vec<_>>()
        );
        assert_eq!(Kind1Codec.decode(&signed).unwrap(), ev);
    }

    #[test]
    fn mention_vs_activity_disambiguation() {
        let keys = Keys::generate();
        // A note WITHOUT a p tag decodes as Activity.
        let act = DomainEvent::Activity(Activity {
            agent: agent(&keys, "coder"),
            project: "p".into(),
            text: "doing stuff".into(),
        });
        assert!(matches!(roundtrip(act, &keys), DomainEvent::Activity(_)));
    }

    #[test]
    fn unrelated_kind_decodes_to_none() {
        let keys = Keys::generate();
        let reaction = EventBuilder::new(Kind::from(7u16), "+")
            .sign_with_keys(&keys)
            .unwrap();
        assert!(Kind1Codec.decode(&reaction).is_none());
    }

    #[test]
    fn kind_24011_presence_is_ignored() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(24011u16), "")
            .tags([
                tag(&["h", "tenex-edge"]).unwrap(),
                tag(&["session-id", "sess-123"]).unwrap(),
                tag(&["agent", &keys.public_key().to_hex(), "coder"]).unwrap(),
                tag(&["expiration", "1900000000"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(Kind1Codec.decode(&event).is_none());
    }

    #[test]
    fn t_only_project_notes_are_ignored() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(KIND_NOTE), "old shape")
            .tags([
                tag(&["t", "tenex-edge"]).unwrap(),
                tag(&["agent", &keys.public_key().to_hex(), "coder"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(Kind1Codec.decode(&event).is_none());
    }

    #[test]
    fn filters_cover_all_kinds_and_mentions() {
        let me = Keys::generate().public_key().to_hex();
        let scope = SubScope {
            authors: vec![Keys::generate().public_key().to_hex()],
            project: Some("tenex-edge".into()),
            mentions_to: Some(me),
            owners: vec![Keys::generate().public_key().to_hex()],
        };
        let filters = Kind1Codec.filters(&scope);
        // profiles, presence/status, notes, mentions-to-me, owner-discovery,
        // NIP-29 group-state (39000/39001/39002 by #d).
        assert_eq!(filters.len(), 6);
        let json = serde_json::to_string(&filters).unwrap();
        assert!(json.contains("\"#h\""));
        assert!(!json.contains("\"#t\""));
        // group-state filter present: addressable kinds scoped by #d=slug.
        assert!(json.contains("\"#d\""));
        assert!(json.contains("39002"));
    }

    #[test]
    fn group_create_has_h_tag() {
        let b = group_create("tenex-edge").unwrap();
        let ev = b.sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_CREATE);
        assert!(has_tag(&ev, "h", "tenex-edge"));
    }

    #[test]
    fn group_lock_closed_is_closed_and_public() {
        let b = group_lock_closed("tenex-edge").unwrap();
        let ev = b.sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_EDIT_METADATA);
        assert!(has_tag(&ev, "h", "tenex-edge"));
        assert!(has_tag(&ev, "name", "tenex-edge"));
        assert!(has_tag_name(&ev, "closed"));
        assert!(has_tag_name(&ev, "public"));
        // Must NOT be private — would blind the non-member daemon connection.
        assert!(!has_tag_name(&ev, "private"));
    }

    #[test]
    fn group_put_user_tags_member() {
        let member = Keys::generate().public_key().to_hex();
        let b = group_put_user("tenex-edge", &member).unwrap();
        let ev = b.sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_PUT_USER);
        assert!(has_tag(&ev, "h", "tenex-edge"));
        // p tag carries the member pubkey with the "member" role.
        assert!(ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some("p")
                && s.get(1).map(String::as_str) == Some(member.as_str())
                && s.get(2).map(String::as_str) == Some("member")
        }));
    }
}
