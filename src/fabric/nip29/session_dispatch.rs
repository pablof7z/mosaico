//! Cross-backend session dispatch event.
//!
//! A dispatch is a kind:9 chat event addressed to a backend management pubkey.
//! Its body is deliberately human-readable, while receivers rely only on tags.
//! The actual handoff message is not included; the requester waits for the new
//! session's kind:30315 ACK, then sends the message p-tagged to that session.

use crate::fabric::nip29::wire::{kind, KIND_CHAT, KIND_STATUS};
use anyhow::Result;
use nostr::*;

pub const MOSAICO_OP_SESSION_DISPATCH: &str = "session.dispatch.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchTarget {
    pub backend_pubkey: String,
    pub slug: String,
    pub workspace: String,
    pub channels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDispatchOp {
    pub route_channel: String,
    pub target: DispatchTarget,
}

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

pub fn build_session_dispatch_event(
    route_channel: &str,
    target: &DispatchTarget,
    prose: &str,
) -> Result<EventBuilder> {
    let mut tags = vec![
        tag(&["h", route_channel])?,
        tag(&["mosaico-op", MOSAICO_OP_SESSION_DISPATCH])?,
        tag(&["p", &target.backend_pubkey])?,
        tag(&["dispatch", &target.backend_pubkey, &target.slug])?,
        tag(&["workspace", &target.workspace])?,
    ];
    for channel in &target.channels {
        tags.push(tag(&["channel", channel])?);
    }
    Ok(EventBuilder::new(kind(KIND_CHAT), prose)
        .tags(tags)
        .allow_self_tagging())
}

pub fn parse_session_dispatch(event: &Event) -> Option<SessionDispatchOp> {
    if event.kind.as_u16() != KIND_CHAT {
        return None;
    }
    let mut mosaico_op = None;
    let mut route_channels = Vec::new();
    let mut dispatches: Vec<(&str, &str)> = Vec::new();
    let mut workspace = None;
    let mut channels = Vec::new();

    for t in event.tags.iter() {
        let s = t.as_slice();
        match s.first().map(String::as_str) {
            Some("mosaico-op") => mosaico_op = s.get(1).map(String::as_str),
            Some("h") => {
                if let Some(v) = s.get(1) {
                    route_channels.push(v.as_str());
                }
            }
            Some("dispatch") => {
                if let (Some(pk), Some(slug)) = (s.get(1), s.get(2)) {
                    dispatches.push((pk, slug));
                }
            }
            Some("workspace") => workspace = s.get(1).map(String::as_str),
            Some("channel") => {
                if let Some(v) = s.get(1).filter(|v| !v.is_empty()) {
                    channels.push(v.clone());
                }
            }
            _ => {}
        }
    }

    if mosaico_op != Some(MOSAICO_OP_SESSION_DISPATCH) {
        return None;
    }
    let route_channel = match route_channels.as_slice() {
        [h] => h.to_string(),
        _ => return None,
    };
    let (backend_pubkey, slug) = match dispatches.as_slice() {
        [(pk, slug)] if !pk.is_empty() && !slug.is_empty() => (*pk, *slug),
        _ => return None,
    };
    let workspace = workspace.filter(|w| !w.is_empty())?.to_string();
    if channels.is_empty() {
        channels.push(workspace.clone());
    }

    Some(SessionDispatchOp {
        route_channel,
        target: DispatchTarget {
            backend_pubkey: backend_pubkey.to_string(),
            slug: slug.to_string(),
            workspace,
            channels,
        },
    })
}

pub fn dispatch_ack_ref(event: &Event) -> Option<&str> {
    if event.kind.as_u16() != KIND_STATUS {
        return None;
    }
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        match (s.first().map(String::as_str), s.get(1)) {
            (Some("e"), Some(id)) => Some(id.as_str()),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(b: EventBuilder) -> Event {
        b.sign_with_keys(&Keys::generate()).unwrap()
    }

    #[test]
    fn build_parse_dispatch_round_trip() {
        let target = DispatchTarget {
            backend_pubkey: "backendpk".into(),
            slug: "codex".into(),
            workspace: "project2".into(),
            channels: vec!["project2.qa".into(), "project1.bug-123".into()],
        };
        let ev = sign(build_session_dispatch_event("project1.bug-123", &target, "x").unwrap());
        let op = parse_session_dispatch(&ev).expect("dispatch parses");

        assert_eq!(op.route_channel, "project1.bug-123");
        assert_eq!(op.target, target);
    }

    #[test]
    fn parse_dispatch_defaults_empty_channel_list_to_workspace() {
        let target = DispatchTarget {
            backend_pubkey: "backendpk".into(),
            slug: "codex".into(),
            workspace: "project2".into(),
            channels: Vec::new(),
        };
        let ev = sign(build_session_dispatch_event("shared", &target, "x").unwrap());
        let op = parse_session_dispatch(&ev).unwrap();

        assert_eq!(op.target.channels, vec!["project2".to_string()]);
    }

    #[test]
    fn dispatch_ack_ref_reads_status_e_tag() {
        let ev = sign(EventBuilder::new(kind(KIND_STATUS), "").tags([tag(&["e", "abc"]).unwrap()]));
        assert_eq!(dispatch_ack_ref(&ev), Some("abc"));
    }
}
