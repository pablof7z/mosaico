use super::*;
use crate::state::{Session, Store};

mod public_session;
pub(super) use public_session::resolve as resolve_public_session;

/// Everything a caller knows about "which session am I" — one envelope, resolved
/// by ONE function, so every in-session command identifies its session the same
/// way every time.
///
/// Public identity is the session pubkey. Hosted sessions expose a typed PTY
/// locator from process birth, recorded at session-start. Native harness shells
/// outside `mosaico agents` use the
/// watched harness process (`watch_pid`) as their exact anchor.
/// `harness_session` covers harness-native resume locators reported by hooks.
#[derive(Default, Clone, Copy)]
pub(in crate::daemon::server) struct CallerAnchor<'a> {
    /// `--session` operator override: npub, hex pubkey, or current handle.
    pub explicit: Option<&'a str>,
    /// The hosted PTY session the caller runs in, resolved via its alias.
    pub pty_session: Option<&'a str>,
    /// Harness-native session id reported by a hook, resolved via its
    /// `harness_session` alias. Never the identity — only a locator.
    pub harness_session: Option<&'a str>,
    /// Watched harness process recorded by session-start. This is the exact
    /// native-harness-shell anchor when there is no hosted PTY session.
    pub watch_pid: Option<i32>,
    /// The harness that owns `harness_session` (e.g. `claude-code`). The alias
    /// table is keyed by `(harness, kind, external_id)`, so this full-keys the
    /// harness-session lookup — a harness-native id is only unique WITHIN its
    /// harness. (`pty_session` needs no harness.)
    pub harness: Option<&'a str>,
}

impl<'a> CallerAnchor<'a> {
    /// Build the anchor from raw RPC params — the SINGLE place wire field names
    /// map to anchor fields, so every handler resolves identically (SSOT).
    /// Borrows from `p`, so `p` must outlive the anchor. NOTE: the caller's
    /// agent slug is the `agent` key; `invite`
    /// carries two agents and builds its anchor by hand.
    pub(in crate::daemon::server) fn from_params(p: &'a serde_json::Value) -> Self {
        let s = |k: &str| p.get(k).and_then(|v| v.as_str()).filter(|x| !x.is_empty());
        let pid = |k: &str| {
            p.get(k).and_then(|v| {
                v.as_i64()
                    .and_then(|n| i32::try_from(n).ok())
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
        };
        CallerAnchor {
            explicit: s("session").or_else(|| s("pubkey")),
            pty_session: s("pty_session"),
            harness_session: s("harness_session"),
            watch_pid: pid("watch_pid"),
            harness: s("harness"),
        }
    }
}

/// How far resolution may reach past the exact anchors before failing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::server) enum ResolveScope {
    /// Exact anchors only (explicit / PTY session / harness id). No cwd+agent scan.
    /// Fails loud rather than binding a sibling. For per-session MUTATIONS
    /// (channel switch/join/leave, invite, create) where guessing the wrong
    /// session is harmful.
    Strict,
    /// Exact anchors, then the cwd+agent scan (latest-alive in the channel). For
    /// reads and host-facing commands (who/turn/chat/propose) run from a repo.
    Channel,
}

pub(in crate::daemon::server) fn resolve_session(
    state: &Arc<DaemonState>,
    anchor: &CallerAnchor,
) -> Result<Session> {
    resolve_session_inner(state, anchor, ResolveScope::Channel)
}

/// The root channel a routing scope belongs under: a top-level channel is its
/// own work root; sub-channels walk to the top-level channel root.
pub(in crate::daemon::server) fn work_root_for(s: &Store, scope: &str) -> String {
    s.root_channel_of(scope)
        .ok()
        .flatten()
        .unwrap_or_else(|| scope.to_string())
}

/// Resolve the caller's session through the single priority order:
///   1. explicit `--session` (operator/host override; may name a dead session)
///   2. PTY session alias  (live only)
///   3. harness-session alias  (live only)
///   4. watch pid alias  (live only)
///
/// Typed locators never become public selectors and never return a dead row.
pub(in crate::daemon::server) fn resolve_session_inner(
    state: &Arc<DaemonState>,
    anchor: &CallerAnchor,
    scope: ResolveScope,
) -> Result<Session> {
    // 1. Explicit `--session`: public operator identity only.
    if let Some(selector) = anchor.explicit.filter(|s| !s.is_empty()) {
        return state
            .with_store(|s| resolve_public_session(s, selector))?
            .with_context(|| {
                format!("unknown public session {selector:?}; use an npub, hex pubkey, or handle")
            });
    }
    // 2. Hosted PTY endpoint.
    if let Some(pty_session) = anchor.pty_session.filter(|s| !s.is_empty()) {
        if let Some(rec) = state
            .with_store(|s| {
                s.alive_session_for_locator(None, crate::state::LOCATOR_PTY, pty_session)
            })
            .ok()
            .flatten()
        {
            return Ok(rec);
        }
    }
    // 3. Harness-native resume locator reported by a hook (live only).
    if let Some(hs) = anchor.harness_session.filter(|s| !s.is_empty()) {
        let harness = anchor
            .harness
            .map(|h| crate::session::Harness::from_str(h).as_str());
        if let Some(rec) = state
            .with_store(|s| {
                s.alive_session_for_locator(harness, crate::state::LOCATOR_NATIVE_RESUME, hs)
            })
            .ok()
            .flatten()
        {
            return Ok(rec);
        }
    }
    // 4. Watched harness process — exact for native Claude/Codex/Grok shells
    // that were not launched through mosaico and therefore lack a PTY anchor.
    if let (Some(pid), Some(harness)) = (anchor.watch_pid, anchor.harness) {
        let harness = crate::session::Harness::from_str(harness).as_str();
        let pid = pid.to_string();
        if let Some(rec) = state
            .with_store(|s| {
                s.alive_session_for_locator(Some(harness), crate::state::LOCATOR_PID, &pid)
            })
            .ok()
            .flatten()
        {
            return Ok(rec);
        }
    }
    let _ = scope;
    anyhow::bail!(
        "must run inside a registered mosaico session or pass an npub, hex pubkey, or handle"
    )
}

#[cfg(test)]
mod tests {
    use super::work_root_for;
    use crate::state::Store;

    #[test]
    fn work_root_for_walks_to_top_level_root() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        store.upsert_channel("task", "Task", "", "root", 1).unwrap();
        store.upsert_channel("deep", "Deep", "", "task", 1).unwrap();

        assert_eq!(work_root_for(&store, "deep"), "root");
        assert_eq!(work_root_for(&store, "root"), "root");
        assert_eq!(work_root_for(&store, "unknown"), "unknown");
    }
}
