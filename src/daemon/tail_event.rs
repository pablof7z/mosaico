//! `TailEvent` — the structured event type that replaces the pre-rendered
//! `{line}` strings on the tail channel.
//!
//! All variants are `Serialize + Deserialize` so they transit the UDS as JSON.
//! The CLI owns all rendering; the daemon only emits raw events.

use serde::{Deserialize, Serialize};

/// One event on the tail stream. Sent as `Response::item(to_value(ev))`.
///
/// The `category` field is the serde tag. Field names match the spec §6.A.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum TailEvent {
    /// Directed message / mention arriving or departing.
    Msg {
        ts: u64,
        project: String,
        from: String,
        from_session: Option<String>,
        to: String,
        to_session: Option<String>,
        body: String,
    },
    /// Outbound delivery state transition.
    Sync {
        ts: u64,
        project: String,
        from: String,
        to: String,
        /// "accepted" | "delivered" | "failed"
        state: String,
        detail: Option<String>,
    },
    /// Working / idle transition for a local session.
    Turn {
        ts: u64,
        project: String,
        agent: String,
        session: String,
        /// "working" | "idle"
        state: String,
        /// Elapsed seconds since the turn started (meaningful on idle).
        elapsed_s: Option<u64>,
    },
    /// NIP-38 status changed: the persistent session title (`text`) and/or the
    /// mid-turn flag (`active`).
    Status {
        ts: u64,
        project: String,
        agent: String,
        text: String,
        /// Whether the session is mid-turn (idle = !active).
        active: bool,
    },
    /// Peer session came online (first-seen heartbeat).
    Join {
        ts: u64,
        project: String,
        agent: String,
        host: String,
        session: String,
        rel_cwd: String,
    },
    /// Peer session went offline (prune / expiry / rpc_session_end).
    Leave {
        ts: u64,
        project: String,
        agent: String,
        host: String,
        session: String,
        /// How long the session was visible to us, in seconds.
        online_s: u64,
    },
    /// Own session start / end.
    Sess {
        ts: u64,
        project: String,
        agent: String,
        session: String,
        /// "start" | "end"
        state: String,
        rel_cwd: String,
    },
    /// Project metadata (about) changed.
    Proj {
        ts: u64,
        project: String,
        about: String,
    },
    /// New agent profile first discovered (default-hidden tier).
    Profile {
        ts: u64,
        agent: String,
        host: String,
        pubkey: String,
    },
}

impl TailEvent {
    pub(crate) fn delivery_failure(
        ts: u64,
        project: &str,
        agent: &str,
        session: &str,
        detail: impl Into<String>,
    ) -> Self {
        TailEvent::Sync {
            ts,
            project: project.to_string(),
            from: "tenex-edge".to_string(),
            to: agent.to_string(),
            state: "failed".to_string(),
            detail: Some(format!("session {session}: {}", detail.into())),
        }
    }

    /// Return the unix timestamp for this event.
    pub fn ts(&self) -> u64 {
        match self {
            TailEvent::Msg { ts, .. } => *ts,
            TailEvent::Sync { ts, .. } => *ts,
            TailEvent::Turn { ts, .. } => *ts,
            TailEvent::Status { ts, .. } => *ts,
            TailEvent::Join { ts, .. } => *ts,
            TailEvent::Leave { ts, .. } => *ts,
            TailEvent::Sess { ts, .. } => *ts,
            TailEvent::Proj { ts, .. } => *ts,
            TailEvent::Profile { ts, .. } => *ts,
        }
    }

    /// Return the severity tier for default display filtering.
    ///
    /// Tiers (high to low):
    ///   2 = signal  (msg, turn, join, leave, sync failed)
    ///   1 = ambient (status, sync delivered/accepted, sess, proj)
    ///   0 = noise   (profile, heartbeats — never emitted)
    pub fn tier(&self) -> u8 {
        match self {
            TailEvent::Msg { .. } => 2,
            TailEvent::Turn { .. } => 2,
            TailEvent::Join { .. } => 2,
            TailEvent::Leave { .. } => 2,
            TailEvent::Sync { state, .. } if state == "failed" => 2,
            TailEvent::Status { .. } => 1,
            TailEvent::Sync { .. } => 1, // accepted / delivered
            TailEvent::Sess { .. } => 1,
            TailEvent::Proj { .. } => 1,
            TailEvent::Profile { .. } => 0,
        }
    }

    /// Return the category name for --only/--exclude filtering.
    pub fn category(&self) -> &'static str {
        match self {
            TailEvent::Msg { .. } => "msg",
            TailEvent::Sync { .. } => "sync",
            TailEvent::Turn { .. } => "turn",
            TailEvent::Status { .. } => "stat",
            TailEvent::Join { .. } => "join",
            TailEvent::Leave { .. } => "leave",
            TailEvent::Sess { .. } => "sess",
            TailEvent::Proj { .. } => "proj",
            TailEvent::Profile { .. } => "profile",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ordering_is_correct() {
        let msg = TailEvent::Msg {
            ts: 0,
            project: "proj".into(),
            from: "a".into(),
            from_session: None,
            to: "b".into(),
            to_session: None,
            body: "hi".into(),
        };
        let status = TailEvent::Status {
            ts: 0,
            project: "proj".into(),
            agent: "a".into(),
            text: "working".into(),
            active: true,
        };
        let profile = TailEvent::Profile {
            ts: 0,
            agent: "a".into(),
            host: "h".into(),
            pubkey: "p".into(),
        };
        assert_eq!(msg.tier(), 2);
        assert_eq!(status.tier(), 1);
        assert_eq!(profile.tier(), 0);
    }

    #[test]
    fn category_names_match_spec() {
        let ev = TailEvent::Join {
            ts: 0,
            project: "p".into(),
            agent: "a".into(),
            host: "h".into(),
            session: "s".into(),
            rel_cwd: ".".into(),
        };
        assert_eq!(ev.category(), "join");
    }

    #[test]
    fn delivery_failure_is_failed_sync_with_session_detail() {
        let ev = TailEvent::delivery_failure(7, "chan", "codex", "sid-1", "endpoint vanished");
        match ev {
            TailEvent::Sync {
                ts,
                project,
                from,
                to,
                state,
                detail,
            } => {
                assert_eq!(ts, 7);
                assert_eq!(project, "chan");
                assert_eq!(from, "tenex-edge");
                assert_eq!(to, "codex");
                assert_eq!(state, "failed");
                assert_eq!(detail.as_deref(), Some("session sid-1: endpoint vanished"));
            }
            _ => panic!("expected sync event"),
        }
    }

    #[test]
    fn roundtrip_serialization() {
        let ev = TailEvent::Turn {
            ts: 1_700_000_000,
            project: "tenex-edge".into(),
            agent: "claude".into(),
            session: "te-abc-123".into(),
            state: "working".into(),
            elapsed_s: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: TailEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
        // Category tag present
        assert!(json.contains("\"category\":\"turn\""));
    }
}
