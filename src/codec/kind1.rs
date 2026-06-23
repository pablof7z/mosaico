//! The `kind1` codec set — tenex-edge's wire shape (M1 §3).
//!
//! | Domain      | Wire |
//! |-------------|------|
//! | Profile     | kind:0,     content `{"name": slug}`, `["host", host]` |
//! | Activity    | kind:1,     `["h", project]` — social narrative (no inbox routing) |
//! | Status      | kind:30315, content = live activity (may be empty when idle), `["d", group_id]`, `["h", group_id]` (`d == h == project slug`, the durable agent's group), `["title", title]` (always), `["status", "busy"\|"idle"]`, `["host", host]`, optional `["rel-cwd", rel]`, optional NIP-40 `["expiration", ts]` |
//! | Chat        | kind:9,     `["h", project]`, optional `["from-session", sender]`, optional `["p", mentioned_pubkey]`, optional `["session-id", mentioned_session]` |
//!
//! Status is the single self-contained per-group signal: ONE kind:30315 event
//! per `(author_pubkey, group_id)` carries the whole live state (busy/idle, the
//! live activity in the content, the persistent title, host, rel-cwd). It is
//! replaceable PER GROUP via `d = group_id` (the project slug), with the hard
//! invariant `d == h` enforced on both encode and decode. Events where `d != h`
//! are rejected as malformed. Liveness IS the freshness of this event: the daemon
//! re-arms a NIP-40 `["expiration", now + STATUS_TTL_SECS]` tag on every
//! heartbeat, so a stopped session's event ages off the relay shortly after its
//! last beat. A `Status` with `expires_at == None` publishes no expiration (tests
//! / non-heartbeat contexts). There is no separate presence heartbeat.
//!
//! Chat (kind:9) is the sole agent-to-agent messaging mechanism. Direct messaging
//! uses an inline `@<codename>` in the chat body, which adds `p` + `session-id`
//! tags to the chat event for the first codename found.
//!
//! Slug is NOT carried on the wire; it is resolved downstream from the signer's
//! kind:0 profile (authoritative) or the local `profiles` table. Authorization
//! uses only event.pubkey (signer); self-asserted `agent` tags have no authority
//! and are never written or read.

use crate::codec::Codec;
use crate::domain::{Activity, AgentRef, ChatMessage, DomainEvent, Profile, Proposal, Status};
use crate::util::SessionId;
use anyhow::Result;
use nostr_sdk::prelude::*;

pub const KIND_PROFILE: u16 = 0;
pub const KIND_CHAT: u16 = 9;
pub const KIND_STATUS: u16 = 30315;

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
            DomainEvent::Activity(Activity {
                agent: _agent,
                project,
                text,
            }) => {
                // Activity is a social narrative note (kind:1 without inbox routing).
                // We still encode it for broadcast purposes but it's not part of
                // the inbox system. For now, encode as a plain kind:1 note.
                use nostr_sdk::prelude::EventBuilder as EB;
                EB::new(Kind::from(1u16), text.clone()).tags([project_tag(project)?])
            }
            DomainEvent::Status(Status {
                agent,
                project,
                session_id: _session_id, // not emitted on kind:30315; d == h == project
                host,
                title,
                activity,
                busy,
                rel_cwd,
                expires_at,
            }) => {
                // The single self-contained per-group signal. `d == h == group_id`
                // (the project slug) makes the event addressable per
                // (author_pubkey, group_id). Content is the live activity (empty
                // when idle); the title always rides as a tag so it persists
                // across idle turns AND after exit. Liveness IS the freshness of
                // this event: when `expires_at` is Some, a NIP-40
                // `["expiration", ts]` tag rides the wire, so a stopped session's
                // event ages off the relay ~STATUS_TTL_SECS after its last
                // heartbeat re-arm. `None` publishes no expiration (tests /
                // non-heartbeat contexts).
                //
                // `d == h == project` — the invariant is enforced here and
                // required on decode. No `session-id` tag on kind:30315 (chat
                // kind:9 keeps its own `session-id`/`from-session` tags).
                let d = project.as_str();
                let mut tags = vec![
                    tag(&["d", d])?,
                    project_tag(project)?,
                    tag(&["title", title])?,
                    tag(&["status", if *busy { "busy" } else { "idle" }])?,
                    tag(&["host", host])?,
                ];
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
                project,
                body,
                from_session,
                mentioned_session,
                mentioned_pubkey,
            }) => {
                let mut tags = vec![project_tag(project)?];
                if let Some(s) = from_session {
                    tags.push(tag(&["from-session", s])?);
                }
                if let Some(pk) = mentioned_pubkey {
                    tags.push(tag(&["p", pk])?);
                }
                if let Some(s) = mentioned_session {
                    tags.push(tag(&["session-id", s])?);
                }
                EventBuilder::new(kind(KIND_CHAT), body.clone())
                    .tags(tags)
                    .allow_self_tagging()
            }
            DomainEvent::Proposal(Proposal {
                agent: _,
                project,
                title,
                body,
                d,
                session_id,
                audience,
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
                // Per-group addressable status: d must equal h (the group_id).
                // Events where d != h are malformed/foreign and are rejected.
                let d = first_tag(event, "d")?;
                let h = first_tag(event, "h")?;
                if d != h {
                    return None;
                }
                let group_id = d.to_string();
                Some(DomainEvent::Status(Status {
                    // Slug rides as a convenience tag (avoids a kind:0 lookup);
                    // empty on legacy emitters, resolved downstream from kind:0.
                    agent: AgentRef::new(
                        pubkey,
                        first_tag(event, "slug").unwrap_or_default().to_string(),
                    ),
                    project: group_id,
                    // session_id is no longer carried on kind:30315. Field is
                    // scheduled for removal from Status in domain.rs; callers
                    // that relied on native_session_id from this decode path must
                    // be updated by the integrator (see materializer.rs).
                    session_id: SessionId::from(""),
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
                project: project_from_tags(event)?,
                body: event.content.clone(),
                from_session: first_tag(event, "from-session").map(str::to_string),
                mentioned_session: first_tag(event, "session-id").map(str::to_string),
                mentioned_pubkey: first_tag(event, "p").map(str::to_string),
            })),
            1 => {
                // kind:1 notes: decode as Activity for social narrative (no routing).
                let project = project_from_tags(event)?;
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

    fn status(keys: &Keys, busy: bool, rel_cwd: &str) -> DomainEvent {
        DomainEvent::Status(Status {
            // Slug is NOT on the wire; decoded status always has empty slug.
            agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
            project: "tenex-edge".into(),
            // session_id is no longer on the kind:30315 wire; decode always
            // yields "" here. Roundtrip tests must use "" to stay equal.
            session_id: "".into(),
            host: "laptop".into(),
            title: "fixing the auth bug".into(),
            activity: if busy {
                "reading the diff".into()
            } else {
                String::new()
            },
            busy,
            rel_cwd: rel_cwd.into(),
            // Default helper builds a non-expiring status; the expiration
            // roundtrip is covered by `status_expiration_roundtrips_and_emits_tag`.
            expires_at: None,
        })
    }

    #[test]
    fn status_roundtrip() {
        let keys = Keys::generate();
        let ev = status(&keys, true, "");
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
    }

    #[test]
    fn status_rel_cwd_roundtrips_and_emits_tag() {
        let keys = Keys::generate();
        let ev = status(&keys, true, "worktree1");
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
        let keys = Keys::generate();
        let ev = status(&keys, false, "");
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(!has_tag_name(&signed, "rel-cwd"));
        match Kind1Codec.decode(&signed) {
            Some(DomainEvent::Status(s)) => assert_eq!(s.rel_cwd, ""),
            other => panic!("expected status, got {other:?}"),
        }
    }

    #[test]
    fn status_is_per_group_self_contained_signal() {
        // The unified shape: `d == h == group_id` (the project slug), full tag
        // set, content = live activity, title persisted as a tag even when busy.
        let keys = Keys::generate();
        let signed = Kind1Codec
            .encode(&status(&keys, true, "worktree1"))
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(signed.kind.as_u16(), KIND_STATUS);
        // Per-GROUP addressable: `d == h == project slug`.
        assert!(has_tag(&signed, "d", "tenex-edge"));
        assert!(has_tag(&signed, "h", "tenex-edge"));
        // d == h is the invariant; no session-id on kind:30315.
        assert!(!has_tag_name(&signed, "session-id"));
        assert!(has_tag(&signed, "title", "fixing the auth bug"));
        assert!(has_tag(&signed, "status", "busy"));
        assert!(has_tag(&signed, "host", "laptop"));
        assert!(has_tag(&signed, "rel-cwd", "worktree1"));
        // A None `expires_at` publishes no NIP-40 expiration tag.
        assert!(!has_tag_name(&signed, "expiration"));
        // The live activity is the content, not a tag.
        assert_eq!(signed.content, "reading the diff");
        assert!(!has_tag_name(&signed, "activity"));
        // No legacy presence-heartbeat artifacts, no self-asserted agent tag.
        assert!(!has_tag(&signed, "d", "tenex-edge-presence:sess-123"));
        assert!(!has_tag_name(&signed, "agent"));
    }

    #[test]
    fn idle_status_marks_idle_and_keeps_title() {
        let keys = Keys::generate();
        let signed = Kind1Codec
            .encode(&status(&keys, false, ""))
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(has_tag(&signed, "status", "idle"));
        // Title persists across idle; content (live activity) is empty.
        assert!(has_tag(&signed, "title", "fixing the auth bug"));
        assert_eq!(signed.content, "");
        match Kind1Codec.decode(&signed) {
            Some(DomainEvent::Status(s)) => {
                assert!(s.is_idle());
                assert_eq!(s.title, "fixing the auth bug");
                assert_eq!(s.activity, "");
            }
            other => panic!("expected status, got {other:?}"),
        }
    }

    #[test]
    fn status_expiration_roundtrips_and_emits_tag() {
        // A Some(expires_at) rides the wire as a NIP-40 `["expiration", ts]` tag
        // and decodes back to the same value — liveness IS this event's freshness.
        let keys = Keys::generate();
        let mut ev = status(&keys, true, "");
        if let DomainEvent::Status(s) = &mut ev {
            s.expires_at = Some(1_900_000_000);
        }
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(has_tag(&signed, "expiration", "1900000000"));
        match Kind1Codec.decode(&signed) {
            Some(DomainEvent::Status(s)) => assert_eq!(s.expires_at, Some(1_900_000_000)),
            other => panic!("expected status, got {other:?}"),
        }
    }

    #[test]
    fn status_old_d_shape_rejected_when_d_ne_h() {
        // Old wire shape `d = "<project>:<session>"` produces d != h, so it must
        // be rejected. This is the tombstone for the old fallback behaviour.
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(KIND_STATUS), "")
            .tags([
                tag(&["h", "tenex-edge"]).unwrap(),
                tag(&["d", "tenex-edge:sess-xyz"]).unwrap(),
                tag(&["status", "idle"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            Kind1Codec.decode(&event).is_none(),
            "d != h must be rejected (old <project>:<session> shape)"
        );
    }

    #[test]
    fn status_d_equals_h_is_accepted() {
        // New canonical shape: `d == h == group_id`.
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(KIND_STATUS), "working on tests")
            .tags([
                tag(&["h", "tenex-edge"]).unwrap(),
                tag(&["d", "tenex-edge"]).unwrap(),
                tag(&["status", "busy"]).unwrap(),
                tag(&["title", "codec refactor"]).unwrap(),
                tag(&["host", "laptop"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        match Kind1Codec.decode(&event) {
            Some(DomainEvent::Status(s)) => {
                assert_eq!(s.project, "tenex-edge");
                assert_eq!(s.activity, "working on tests");
                assert_eq!(s.title, "codec refactor");
                assert!(s.busy);
            }
            other => panic!("expected status, got {other:?}"),
        }
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
        // A kind:1 with only a `t` tag (old hashtag shape, no `h` tag) → None
        // (no `h` tag means no project, so project_from_tags returns None).
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(1u16), "old shape")
            .tags([tag(&["t", "tenex-edge"]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(Kind1Codec.decode(&event).is_none());
    }

    #[test]
    fn chat_message_encodes_as_kind9_with_group_and_mention_tags() {
        let keys = Keys::generate();
        let mentioned_pk = "dd".repeat(32);
        let ev = DomainEvent::ChatMessage(ChatMessage {
            from: agent(&keys, "codex"),
            project: "myproject".into(),
            body: "status: tests are green".into(),
            from_session: Some("sender-sess".into()),
            mentioned_session: Some("target-sess".into()),
            mentioned_pubkey: Some(mentioned_pk.clone()),
        });
        let codec = Kind1Codec;
        let builder = codec.encode(&ev).expect("encode");
        let signed = builder.sign_with_keys(&keys).expect("sign");

        assert_eq!(signed.kind.as_u16(), KIND_CHAT);
        assert!(has_tag(&signed, "h", "myproject"));
        // Chat (kind:9) keeps from-session/session-id as display metadata even
        // though mention (kind:1) routing cut over to the session pubkey.
        assert!(has_tag(&signed, "from-session", "sender-sess"));
        assert!(has_tag(&signed, "session-id", "target-sess"));
        assert!(has_tag(&signed, "p", &mentioned_pk));

        match codec.decode(&signed) {
            Some(DomainEvent::ChatMessage(chat)) => {
                assert_eq!(chat.project, "myproject");
                assert_eq!(chat.body, "status: tests are green");
                assert_eq!(chat.from_session, Some("sender-sess".into()));
                assert_eq!(chat.mentioned_session, Some("target-sess".into()));
                assert_eq!(chat.mentioned_pubkey, Some(mentioned_pk));
            }
            other => panic!("expected ChatMessage, got {other:?}"),
        }
    }
}
