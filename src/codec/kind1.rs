//! The `kind1` codec set — tenex-edge's initial wire shape (M1 §3).
//!
//! | Domain    | Wire |
//! |-----------|------|
//! | Profile   | kind:0,    content `{"name": slug}`, `["host", host]` |
//! | Activity   | kind:1,    `["h", project]` |
//! | TurnReply  | kind:1,    `["h", project]`, `["e", root_id, "", "root"]`, `["e", reply_id, "", "reply"]` |
//! | Status     | kind:30315, content = live activity (may be empty when idle), `["d", "<project>:<session>"]`, `["h", project]`, `["session-id", id]`, `["title", title]` (always), `["status", "busy"\|"idle"]`, `["host", host]`, optional `["rel-cwd", rel]`, optional NIP-40 `["expiration", ts]` |
//! | Mention    | kind:1,    `["h", project]`, `["p", to]`, optional `["session-id", target]`, `["from-session", sender]`, `["subject", s]`, `["git-branch", b]`, `["git-commit", c]`, `["git-dirty", n]`, `["from-host", h]`, `["e", reply_to, "", "reply"]` |
//! | Chat       | kind:9,    `["h", project]`, optional `["from-session", sender]`, optional `["p", mentioned_pubkey]`, optional `["session-id", mentioned_session]` |
//!
//! Status is the single self-contained per-session signal: ONE kind:30315 event
//! carries the whole live state of a session (busy/idle, the live activity in
//! the content, the persistent title, host, rel-cwd). It is replaceable PER
//! SESSION via `d = "<project>:<session>"`, so each session keeps its own title
//! even when idle. Liveness IS the freshness of this event: the daemon re-arms a
//! NIP-40 `["expiration", now + STATUS_TTL_SECS]` tag on every heartbeat, so a
//! stopped session's event ages off the relay shortly after its last beat. A
//! `Status` with `expires_at == None` publishes no expiration (tests /
//! non-heartbeat contexts). There is no separate presence heartbeat.
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
    Activity, AgentRef, ChatMessage, DomainEvent, Mention, MentionMeta, Profile, Proposal, Status,
    TurnReply,
};
use crate::util::SessionId;
use anyhow::Result;
use nostr_sdk::prelude::*;

pub const KIND_PROFILE: u16 = 0;
pub const KIND_NOTE: u16 = 1;
pub const KIND_CHAT: u16 = 9;
pub const KIND_STATUS: u16 = 30315;

// NIP-29 group management (operator/userNsec-signed) + relay-authored state.
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

/// Per-session addressable `d` value: `"<project>:<session>"`. Makes the status
/// event replaceable per session so each session keeps its own title when idle.
fn status_d(project: &str, session_id: &str) -> String {
    format!("{project}:{session_id}")
}

/// Split a `d = "<project>:<session>"` value into its session-id suffix.
/// The project may itself contain colons; the session id is the part after the
/// LAST colon. Returns `None` when there is no colon.
fn session_from_status_d(d: &str) -> Option<&str> {
    d.rsplit_once(':').map(|(_project, session)| session)
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
            DomainEvent::Activity(Activity {
                agent: _agent,
                project,
                text,
            }) => EventBuilder::new(kind(KIND_NOTE), text.clone()).tags([project_tag(project)?]),
            DomainEvent::Status(Status {
                agent: _agent,
                project,
                session_id,
                host,
                title,
                activity,
                busy,
                rel_cwd,
                expires_at,
                thread_root_id,
            }) => {
                // The single self-contained per-session signal. Content is the
                // live activity (empty when idle); the title always rides as a
                // tag so it persists across idle turns AND after exit. Liveness
                // IS the freshness of this event: when `expires_at` is Some, a
                // NIP-40 `["expiration", ts]` tag rides the wire, so a stopped
                // session's event ages off the relay ~STATUS_TTL_SECS after its
                // last heartbeat re-arm. `None` publishes no expiration (tests /
                // non-heartbeat contexts).
                let d = status_d(project, session_id.as_str());
                let mut tags = vec![
                    tag(&["d", &d])?,
                    project_tag(project)?,
                    tag(&["session-id", session_id.as_str()])?,
                    tag(&["title", title])?,
                    tag(&["status", if *busy { "busy" } else { "idle" }])?,
                    tag(&["host", host])?,
                ];
                if !rel_cwd.is_empty() {
                    tags.push(tag(&["rel-cwd", rel_cwd])?);
                }
                if let Some(exp) = expires_at {
                    tags.push(tag(&["expiration", &exp.to_string()])?);
                }
                if let Some(root) = thread_root_id {
                    // NIP-10 root marker → maps this session to its conversation
                    // thread. kind:30315 is decoded by kind number, so this `e`
                    // tag never trips the kind:1 Mention/TurnReply disambiguation.
                    tags.push(tag(&["e", root, "", "root"])?);
                }
                EventBuilder::new(kind(KIND_STATUS), activity.clone()).tags(tags)
            }
            DomainEvent::Mention(Mention {
                from: _from,
                to_pubkey,
                project,
                body,
                meta,
            }) => {
                let mut tags = vec![project_tag(project)?, tag(&["p", to_pubkey])?];
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
                // Every kind:30315 is a single self-contained per-session status.
                // The session id comes from the explicit `session-id` tag, else
                // from the `<project>:<session>` suffix of the `d` tag.
                let session_id = first_tag(event, "session-id")
                    .or_else(|| first_tag(event, "d").and_then(session_from_status_d))?;
                Some(DomainEvent::Status(Status {
                    // Slug is NOT on the wire; resolved downstream from kind:0 profile.
                    agent: AgentRef::new(pubkey, String::new()),
                    project: project_from_tags(event)?,
                    session_id: SessionId::from(session_id),
                    host: first_tag(event, "host").unwrap_or_default().to_string(),
                    title: first_tag(event, "title").unwrap_or_default().to_string(),
                    // The live activity is the event content (empty when idle).
                    activity: event.content.clone(),
                    busy: first_tag(event, "status") == Some("busy"),
                    rel_cwd: first_tag(event, "rel-cwd").unwrap_or_default().to_string(),
                    // NIP-40 expiration → liveness clock. Absent → None.
                    expires_at: first_tag(event, "expiration").and_then(|s| s.parse().ok()),
                    // NIP-10 root marker → the conversation thread this session
                    // opened. Absent on legacy emitters / pre-first-prompt → None.
                    thread_root_id: e_tag_with_marker(event, "root").map(str::to_string),
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

    fn status(keys: &Keys, busy: bool, rel_cwd: &str) -> DomainEvent {
        DomainEvent::Status(Status {
            // Slug is NOT on the wire; decoded status always has empty slug.
            agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
            project: "tenex-edge".into(),
            session_id: "sess-123".into(),
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
            // Root-link roundtrip is covered by a dedicated test below.
            thread_root_id: None,
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
    fn status_is_per_session_self_contained_signal() {
        // The single unified shape: a per-session `d`, the full tag set, content =
        // live activity, and the title persisted as a tag even when busy.
        let keys = Keys::generate();
        let signed = Kind1Codec
            .encode(&status(&keys, true, "worktree1"))
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(signed.kind.as_u16(), KIND_STATUS);
        // Per-SESSION addressable `d = "<project>:<session>"`.
        assert!(has_tag(&signed, "d", "tenex-edge:sess-123"));
        assert!(has_tag(&signed, "h", "tenex-edge"));
        assert!(has_tag(&signed, "session-id", "sess-123"));
        assert!(has_tag(&signed, "title", "fixing the auth bug"));
        assert!(has_tag(&signed, "status", "busy"));
        assert!(has_tag(&signed, "host", "laptop"));
        assert!(has_tag(&signed, "rel-cwd", "worktree1"));
        // A None `expires_at` publishes no NIP-40 expiration tag.
        assert!(!has_tag_name(&signed, "expiration"));
        // The live activity is the content, not a tag.
        assert_eq!(signed.content, "reading the diff");
        assert!(!has_tag_name(&signed, "activity"));
        // No presence-heartbeat artifacts, no self-asserted agent tag.
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
    fn status_thread_root_roundtrips_as_nip10_root_e_tag() {
        // The conversation thread root rides the wire as a NIP-10
        // `["e", root, "", "root"]` tag so a reader can map this session to its
        // kind:1 prompt/reply timeline; it decodes back to the same id.
        let keys = Keys::generate();
        let mut ev = status(&keys, true, "");
        let root = "ab".repeat(32);
        if let DomainEvent::Status(s) = &mut ev {
            s.thread_root_id = Some(root.clone());
        }
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(has_tag(&signed, "e", &root));
        // Absent on a status with no recorded root → None.
        let bare = Kind1Codec
            .encode(&status(&keys, true, ""))
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(!has_tag_name(&bare, "e"));
        match Kind1Codec.decode(&bare) {
            Some(DomainEvent::Status(s)) => assert_eq!(s.thread_root_id, None),
            other => panic!("expected status, got {other:?}"),
        }
    }

    #[test]
    fn status_session_recovered_from_d_when_session_id_tag_absent() {
        // A peer that omits the explicit session-id tag still decodes: the session
        // id is the suffix of `d = "<project>:<session>"`.
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(KIND_STATUS), "")
            .tags([
                tag(&["h", "tenex-edge"]).unwrap(),
                tag(&["d", "tenex-edge:sess-xyz"]).unwrap(),
                tag(&["status", "idle"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        match Kind1Codec.decode(&event) {
            Some(DomainEvent::Status(s)) => {
                assert_eq!(s.session_id.as_str(), "sess-xyz");
                assert_eq!(s.project, "tenex-edge");
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
    fn mention_roundtrip() {
        // Slug is NOT on the wire; decoded mention always has empty slug.
        let keys = Keys::generate();
        let ev = DomainEvent::Mention(Mention {
            from: AgentRef::new(keys.public_key().to_hex(), String::new()),
            to_pubkey: "cc".repeat(32),
            project: "tenex-edge".into(),
            body: "can you review?".into(),
            meta: MentionMeta::default(),
        });
        assert_eq!(roundtrip(ev.clone(), &keys), ev);
        // Wire event must have NO agent tag, no session-id tag, no from-session tag.
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(!has_tag_name(&signed, "agent"));
        assert!(!has_tag_name(&signed, "session-id"));
        assert!(!has_tag_name(&signed, "from-session"));
    }

    #[test]
    fn mention_no_session_tags_on_wire() {
        let keys = Keys::generate();
        // Session-pubkey addressing: no session-id or from-session tags emitted.
        let ev = DomainEvent::Mention(Mention {
            from: AgentRef::new(keys.public_key().to_hex(), String::new()),
            to_pubkey: "cc".repeat(32),
            project: "tenex-edge".into(),
            body: "ping".into(),
            meta: MentionMeta::default(),
        });
        let signed = Kind1Codec
            .encode(&ev)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        // Stage 4: session-id and from-session tags must NOT appear on wire.
        assert!(!has_tag_name(&signed, "from-session"));
        assert!(!has_tag_name(&signed, "session-id"));
        // Wire event must have NO agent tag.
        assert!(!has_tag_name(&signed, "agent"));
    }

    #[test]
    fn mention_uses_nip29_h_tag_not_hashtag() {
        let keys = Keys::generate();
        let ev = DomainEvent::Mention(Mention {
            from: agent(&keys, "coder"),
            to_pubkey: "cc".repeat(32),
            project: "tenex-edge".into(),
            body: "can you review?".into(),
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
    fn owner_note_p_only_decodes_as_mention() {
        // Under the new unified rule, ANY kind:1 with a `p` tag decodes as Mention
        // (including p-only notes without session-id). The routing gate in the
        // materializer then decides whether to admit based on signer identity.
        let owner_keys = Keys::generate();
        let agent_pk = "bb".repeat(32);

        let event = EventBuilder::new(Kind::from(KIND_NOTE), "just doing something")
            .tags([
                tag(&["h", "myproject"]).unwrap(),
                tag(&["p", &agent_pk]).unwrap(),
            ])
            .sign_with_keys(&owner_keys)
            .unwrap();

        match Kind1Codec.decode(&event) {
            Some(DomainEvent::Mention(m)) => {
                assert_eq!(m.to_pubkey, agent_pk);
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
            meta: MentionMeta::default(),
        });
        assert!(matches!(roundtrip(ev, &keys), DomainEvent::Mention(_)));
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
