//! The `kind1` codec set — tenex-edge's initial wire shape (M1 §3).
//!
//! | Domain    | Wire |
//! |-----------|------|
//! | Profile   | kind:0,    content `{"name": slug}`, `["host", host]` |
//! | Presence  | kind:30315 (NIP-38-style heartbeat), `["h", project]`, `["d", "tenex-edge-presence:<session>"]`, `["p", peer]…`, `["session-id", id]`, `["host", host]`, optional `["rel-cwd", rel]`, `["expiration", ts]` |
//! | Activity   | kind:1,    `["h", project]` |
//! | TurnReply  | kind:1,    `["h", project]`, `["e", root_id, "", "root"]`, `["e", reply_id, "", "reply"]` |
//! | Status     | kind:30315 (NIP-38), `["h", project]`, `["d", project]`, optional `["session-id", id]`, optional `["rel-cwd", rel]`, optional `["activity", now]`, `["active", "0"|"1"]`, `["expiration", ts]` |
//! | Mention    | kind:1,    `["h", project]`, `["p", to]`, optional `["session-id", target]`, `["from-session", sender]`, `["subject", s]`, `["git-branch", b]`, `["git-commit", c]`, `["git-dirty", n]`, `["from-host", h]`, `["e", reply_to, "", "reply"]` |
//!
//! kind:1 disambiguation on decode (in priority order):
//!   1. Has `["p", ...]` tag                    → Mention
//!   2. Has `["e", ..., "", "root"]` NIP-10 tag → TurnReply
//!   3. Otherwise                               → Activity
//!
//! Slug is NOT carried on the wire; it is resolved downstream from the signer's
//! kind:0 profile (authoritative) or the local `profiles` table. Authorization
//! uses only event.pubkey (signer); self-asserted `agent` tags have no authority
//! and are never written or read.
//!
//! For kind:30023 long-form articles generated during a session, the same
//! `root_event_id` from the session's TurnReply thread should be e-tagged so the
//! article can be linked back to the conversation that produced it.

use crate::codec::Codec;
use crate::domain::{
    Activity, AgentRef, DomainEvent, Mention, MentionMeta, Presence, Profile, Proposal, Status,
    TurnReply,
};
use crate::util::SessionId;
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

// NIP-23 long-form article — used for agent-authored proposals.
pub const KIND_LONGFORM: u16 = 30023;

const PRESENCE_D_PREFIX: &str = "tenex-edge-presence:";

pub struct Kind1Codec;

pub(crate) fn kind(n: u16) -> Kind {
    Kind::from(n)
}

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

pub(crate) fn h_filter(f: Filter, project: &str) -> Filter {
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
                agent: _agent,
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
                agent: _agent,
                project,
                text,
            }) => EventBuilder::new(kind(KIND_NOTE), text.clone()).tags([project_tag(project)?]),
            DomainEvent::Status(Status {
                agent: _agent,
                project,
                session_id,
                text,
                activity,
                active,
                rel_cwd,
                expires_at,
            }) => {
                let mut tags = vec![project_tag(project)?, tag(&["d", project])?];
                if let Some(session_id) = session_id {
                    tags.push(tag(&["session-id", session_id.as_str()])?);
                }
                if !rel_cwd.is_empty() {
                    tags.push(tag(&["rel-cwd", rel_cwd])?);
                }
                if !activity.is_empty() {
                    tags.push(tag(&["activity", activity])?);
                }
                tags.push(tag(&["active", if *active { "1" } else { "0" }])?);
                if let Some(exp) = expires_at {
                    tags.push(tag(&["expiration", &exp.to_string()])?);
                }
                EventBuilder::new(kind(KIND_STATUS), text.clone()).tags(tags)
            }
            DomainEvent::Mention(Mention {
                from: _from,
                to_pubkey,
                project,
                body,
                target_session,
                from_session,
                meta,
            }) => {
                let mut tags = vec![project_tag(project)?, tag(&["p", to_pubkey])?];
                if let Some(sess) = target_session {
                    tags.push(tag(&["session-id", sess.as_str()])?);
                }
                if let Some(sess) = from_session {
                    tags.push(tag(&["from-session", sess.as_str()])?);
                }
                if !meta.subject.is_empty() {
                    tags.push(tag(&["subject", &meta.subject])?);
                }
                if !meta.branch.is_empty() {
                    tags.push(tag(&["git-branch", &meta.branch])?);
                }
                if !meta.commit.is_empty() {
                    tags.push(tag(&["git-commit", &meta.commit])?);
                }
                if meta.dirty > 0 {
                    tags.push(tag(&["git-dirty", &meta.dirty.to_string()])?);
                }
                if !meta.host.is_empty() {
                    tags.push(tag(&["from-host", &meta.host])?);
                }
                if let Some(reply_to) = &meta.reply_to_event_id {
                    // NIP-10 reply marker back to the original mention; the `p`
                    // tag above still makes this decode as a Mention (priority 1).
                    tags.push(tag(&["e", reply_to, "", "reply"])?);
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
            DomainEvent::Proposal(Proposal {
                agent: _,
                project,
                title,
                body,
                d,
                session_id,
                audience,
                thread_root_key,
            }) => {
                let mut tags = vec![
                    tag(&["d", d])?,
                    tag(&["title", title])?,
                    project_tag(project)?,
                    // No agent tag: author identity is the event signer; slug is in kind:0.
                ];
                // Authoring session tag — only when a live session exists.
                if let Some(sess) = session_id {
                    tags.push(tag(&["session-id", sess.as_str()])?);
                }
                // p-tag each owner so the proposal surfaces to the human.
                for owner in audience {
                    tags.push(tag(&["p", owner])?);
                }
                // Thread root e-tag (NIP-10 root marker) — links to the conversation.
                if let Some(root) = thread_root_key {
                    tags.push(tag(&["e", root, "", "root"])?);
                }
                EventBuilder::new(kind(KIND_LONGFORM), body.clone()).tags(tags)
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
                let d = first_tag(event, "d").unwrap_or_default();
                if d.starts_with(PRESENCE_D_PREFIX) {
                    let session_id = first_tag(event, "session-id")?;
                    Some(DomainEvent::Presence(Presence {
                        // Slug is NOT on the wire; resolved downstream from kind:0 profile.
                        agent: AgentRef::new(pubkey, String::new()),
                        project: project_from_tags(event)?,
                        session_id: SessionId::from(session_id),
                        host: first_tag(event, "host").unwrap_or_default().to_string(),
                        rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                        audience: all_tag_values(event, "p"),
                        expires_at: expires_at?,
                    }))
                } else {
                    Some(DomainEvent::Status(Status {
                        // Slug is NOT on the wire; resolved downstream from kind:0 profile.
                        agent: AgentRef::new(pubkey, String::new()),
                        project: project_from_tags(event)?,
                        session_id: first_tag(event, "session-id").map(SessionId::from),
                        text: event.content.clone(),
                        activity: first_tag(event, "activity").unwrap_or_default().to_string(),
                        active: first_tag(event, "active") == Some("1"),
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
                // Authorization is by signer pubkey only; self-asserted agent tags
                // are not written and not read. Slug is always empty here; it is
                // resolved from the profiles table downstream by the materializer.
                if let Some(to) = first_tag(event, "p") {
                    return Some(DomainEvent::Mention(Mention {
                        from: AgentRef::new(pubkey, String::new()),
                        to_pubkey: to.to_string(),
                        project,
                        body: event.content.clone(),
                        target_session: first_tag(event, "session-id").map(SessionId::from),
                        from_session: first_tag(event, "from-session").map(SessionId::from),
                        meta: MentionMeta {
                            subject: first_tag(event, "subject").unwrap_or_default().to_string(),
                            branch: first_tag(event, "git-branch")
                                .unwrap_or_default()
                                .to_string(),
                            commit: first_tag(event, "git-commit")
                                .unwrap_or_default()
                                .to_string(),
                            dirty: first_tag(event, "git-dirty")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0),
                            host: first_tag(event, "from-host")
                                .unwrap_or_default()
                                .to_string(),
                            reply_to_event_id: e_tag_with_marker(event, "reply")
                                .map(|s| s.to_string()),
                        },
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

                // No p tag → Activity.
                Some(DomainEvent::Activity(Activity {
                    agent: AgentRef::new(pubkey, String::new()),
                    project,
                    text: event.content.clone(),
                }))
            }
            KIND_LONGFORM => Some(DomainEvent::Proposal(Proposal {
                // Slug is NOT on the wire; resolved downstream from kind:0 profile.
                agent: AgentRef::new(pubkey, String::new()),
                project: project_from_tags(event)?,
                title: first_tag(event, "title").unwrap_or_default().to_string(),
                body: event.content.clone(),
                d: first_tag(event, "d").unwrap_or_default().to_string(),
                session_id: first_tag(event, "session-id").map(SessionId::from),
                audience: all_tag_values(event, "p"),
                thread_root_key: e_tag_with_marker(event, "root").map(str::to_string),
            })),
            _ => None,
        }
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
        // Slug is NOT on the wire; decoded presence always has empty slug.
        let keys = Keys::generate();
        let ev = DomainEvent::Presence(Presence {
            agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
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
        // Slug is NOT on the wire; decoded presence always has empty slug.
        let keys = Keys::generate();
        let ev = DomainEvent::Presence(Presence {
            agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
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
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(has_tag(&signed, "rel-cwd", "worktree1"));
        // … and the wire event has NO agent tag.
        assert!(!has_tag_name(&signed, "agent"));
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
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
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
        // Slug is NOT on the wire; decoded activity always has empty slug.
        let keys = Keys::generate();
        let ev = DomainEvent::Activity(Activity {
            agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
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
        // Slug is NOT on the wire; decoded status always has empty slug.
        let keys = Keys::generate();
        let ev = DomainEvent::Status(Status {
            agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
            project: "tenex-edge".into(),
            session_id: Some(SessionId::from("sess-status")),
            text: "reviewing PR".into(),
            activity: "reading the diff".into(),
            active: true,
            rel_cwd: String::new(),
            expires_at: Some(1_900_000_000),
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
    }

    #[test]
    fn mention_roundtrip_session_targeted() {
        // Slug is NOT on the wire; decoded mention always has empty slug.
        let keys = Keys::generate();
        let ev = DomainEvent::Mention(Mention {
            from: AgentRef::new(keys.public_key().to_hex(), String::new()),
            to_pubkey: "cc".repeat(32),
            project: "tenex-edge".into(),
            body: "can you review?".into(),
            target_session: Some("sess-xyz".into()),
            // Distinct from target_session so the roundtrip proves they don't swap.
            from_session: Some("sender-sess-1".into()),
            meta: MentionMeta::default(),
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
        // Wire event must have NO agent tag.
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(!has_tag_name(&signed, "agent"));
    }

    #[test]
    fn mention_emits_from_session_tag_and_back_compat_decodes_none() {
        let keys = Keys::generate();
        // With a sender session → a `from-session` tag rides the wire.
        let ev = DomainEvent::Mention(Mention {
            from: AgentRef::new(keys.public_key().to_hex(), String::new()),
            to_pubkey: "cc".repeat(32),
            project: "tenex-edge".into(),
            body: "ping".into(),
            target_session: None,
            from_session: Some("sender-9".into()),
            meta: MentionMeta::default(),
        });
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(has_tag(&signed, "from-session", "sender-9"));
        // Wire event must have NO agent tag.
        assert!(!has_tag_name(&signed, "agent"));

        // A peer note WITHOUT the from-session tag decodes to `from_session: None`.
        let no_from_session = EventBuilder::new(Kind::from(KIND_NOTE), "ping")
            .tags([
                tag(&["h", "tenex-edge"]).unwrap(),
                tag(&["p", &"cc".repeat(32)]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        match Kind1Codec.decode(&no_from_session) {
            Some(DomainEvent::Mention(m)) => assert_eq!(m.from_session, None),
            other => panic!("expected mention, got {other:?}"),
        }
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
            from_session: None,
            meta: MentionMeta::default(),
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
        // Slug is NOT on the wire; decoded mention has empty slug.
        let keys = Keys::generate();
        let pk = keys.public_key().to_hex();
        let ev = DomainEvent::Mention(Mention {
            from: AgentRef::new(pk.clone(), String::new()),
            to_pubkey: pk.clone(),
            project: "p".into(),
            body: "hi".into(),
            target_session: Some("s2".into()),
            from_session: Some("s1".into()),
            meta: MentionMeta::default(),
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
        // No agent tag on the wire.
        assert!(!has_tag_name(&signed, "agent"));
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
                tag(&["expiration", "1900000000"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(Kind1Codec.decode(&event).is_none());
    }

    #[test]
    fn t_only_project_notes_are_ignored() {
        // A kind:1 with only a `t` tag (old hashtag shape, no `h` tag) → None.
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(KIND_NOTE), "old shape")
            .tags([tag(&["t", "tenex-edge"]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(Kind1Codec.decode(&event).is_none());
    }

    // ── Owner-note decode rules ───────────────────────────────────────────────

    #[test]
    fn owner_note_with_p_and_session_id_decodes_as_mention() {
        // A human-signed kind:1 with `p` + `session-id` + NO `agent` tag must
        // decode as a Mention so the materializer can route it via the owner gate.
        let owner_keys = Keys::generate();
        let agent_pk = "aa".repeat(32);
        let session = "sess-owner-1";

        let event = EventBuilder::new(Kind::from(KIND_NOTE), "looks good, ship it")
            .tags([
                tag(&["h", "myproject"]).unwrap(),
                tag(&["p", &agent_pk]).unwrap(),
                tag(&["session-id", session]).unwrap(),
            ])
            .sign_with_keys(&owner_keys)
            .unwrap();

        match Kind1Codec.decode(&event) {
            Some(DomainEvent::Mention(m)) => {
                assert_eq!(m.to_pubkey, agent_pk, "to_pubkey must be the p tag");
                assert_eq!(
                    m.target_session,
                    Some(SessionId::from(session)),
                    "target_session must be the session-id tag"
                );
                assert_eq!(
                    m.from.pubkey,
                    owner_keys.public_key().to_hex(),
                    "from.pubkey must be the event author"
                );
                assert_eq!(m.project, "myproject");
                assert_eq!(m.body, "looks good, ship it");
            }
            other => panic!("expected Mention, got {other:?}"),
        }
    }

    #[test]
    fn owner_note_p_only_no_session_id_decodes_as_mention() {
        // Under the new unified rule, ANY kind:1 with a `p` tag decodes as Mention
        // (including p-only notes without session-id). The routing gate in the
        // materializer then decides whether to admit based on signer identity.
        let owner_keys = Keys::generate();
        let agent_pk = "bb".repeat(32);

        let event = EventBuilder::new(Kind::from(KIND_NOTE), "just doing something")
            .tags([
                tag(&["h", "myproject"]).unwrap(),
                tag(&["p", &agent_pk]).unwrap(),
                // deliberately no session-id
            ])
            .sign_with_keys(&owner_keys)
            .unwrap();

        match Kind1Codec.decode(&event) {
            Some(DomainEvent::Mention(m)) => {
                assert_eq!(m.to_pubkey, agent_pk);
                assert_eq!(m.target_session, None, "no session-id tag → None");
            }
            other => panic!("expected Mention for p-note, got {other:?}"),
        }
    }

    #[test]
    fn p_note_decodes_as_mention_regardless_of_other_tags() {
        // Under the new unified rule, any kind:1 with a `p` tag decodes as Mention.
        // Slug is NOT on the wire; decoded mention has empty slug.
        let keys = Keys::generate();
        let ev = DomainEvent::Mention(Mention {
            from: AgentRef::new(keys.public_key().to_hex(), String::new()),
            to_pubkey: "cc".repeat(32),
            project: "myproject".into(),
            body: "review this".into(),
            target_session: Some("target-sess".into()),
            from_session: None,
            meta: MentionMeta::default(),
        });
        assert!(matches!(roundtrip(ev, &keys), DomainEvent::Mention(_)));
    }
}
