use super::*;
use crate::state::{Session, Store};

/// Everything a caller knows about "which session am I" — one envelope, resolved
/// by ONE function, so every in-session command identifies its session the same
/// way every time.
///
/// The canonical session id is daemon-minted only AFTER the harness process
/// starts, so it can never be a launch-time env var. Hosted sessions expose a
/// PTY session id from process birth, recorded as a `pty_session` alias at
/// session-start. Native harness shells outside tenex-edge launch use the
/// watched harness process (`watch_pid`) as their exact anchor.
/// `harness_session` covers harness-native ids (claude-code/codex report one via
/// hooks; opencode does not). `cwd`+`agent`+`group` are the scan keys — used only
/// outside `Strict`.
#[derive(Default, Clone, Copy)]
pub(in crate::daemon::server) struct CallerAnchor<'a> {
    /// `--session` operator/host override (exact: canonical id or alias).
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
    /// Scan keys (used only in `Channel` scope).
    pub cwd: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub group: Option<&'a str>,
}

impl<'a> CallerAnchor<'a> {
    /// Build the anchor from raw RPC params — the SINGLE place wire field names
    /// map to anchor fields, so every handler resolves identically (SSOT). The
    /// `env_session` key is still read as a `harness_session` alias for
    /// back-compat with older hook senders. Borrows from `p`, so `p` must outlive
    /// the anchor. NOTE: the caller's agent slug is the `agent` key; `invite`
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
            explicit: s("session"),
            pty_session: s("pty_session"),
            harness_session: s("harness_session").or_else(|| s("env_session")),
            watch_pid: pid("watch_pid"),
            harness: s("harness"),
            cwd: s("cwd"),
            agent: s("agent"),
            group: s("group"),
        }
    }
}

/// How far resolution may reach past the exact anchors before failing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::server) enum ResolveScope {
    /// Exact anchors only (explicit / PTY session / harness id). No cwd+agent scan.
    /// Fails loud rather than binding a sibling. For per-session MUTATIONS
    /// (channels switch/join/leave, invite, create) where guessing the wrong
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
///   5. cwd+agent scan  (only outside `Strict`)
///
/// The exact anchors (2,3) resolve through `alive_session_for_alias_kind`, which
/// matches the alias KIND (not just the raw id) and never returns a dead row —
/// so a stale endpoint/harness alias whose owner exited cannot bind a ghost.
pub(in crate::daemon::server) fn resolve_session_inner(
    state: &Arc<DaemonState>,
    anchor: &CallerAnchor,
    scope: ResolveScope,
) -> Result<Session> {
    // 1. Explicit `--session`: operator/host override. May
    //    target a dead session deliberately (resume), so it is not alive-gated.
    if let Some(id) = anchor.explicit.filter(|s| !s.is_empty()) {
        return state
            .with_store(|s| s.get_session(id))
            .with_context(|| format!("unknown session {id}"))?
            .with_context(|| format!("unknown session {id}"));
    }
    // 2. Hosted PTY session — THE in-session anchor for hosted launches.
    if let Some(pty_session) = anchor.pty_session.filter(|s| !s.is_empty()) {
        if let Some(rec) = state
            .with_store(|s| s.alive_session_for_alias(None, "pty_session", pty_session))
            .ok()
            .flatten()
        {
            return Ok(rec);
        }
    }
    // 3. Harness-native session id reported by a hook (live only). Full-keyed by
    //    harness — a harness id is only unique within its own harness. Canonicalize
    //    the harness the SAME way session-start does before STORING the alias
    //    (`Harness::from_str(..).as_str()`), so lookup and storage always agree
    //    even for a name not in the enum (both normalize to "unknown").
    if let Some(hs) = anchor.harness_session.filter(|s| !s.is_empty()) {
        let harness = anchor
            .harness
            .map(|h| crate::session::Harness::from_str(h).as_str());
        if let Some(rec) = state
            .with_store(|s| s.alive_session_for_alias(harness, "harness_session", hs))
            .ok()
            .flatten()
        {
            return Ok(rec);
        }
    }
    // 4. Watched harness process — exact for native Claude/Codex/Grok shells
    // that were not launched through tenex-edge and therefore lack a PTY anchor.
    if let (Some(pid), Some(harness)) = (anchor.watch_pid, anchor.harness) {
        let harness = crate::session::Harness::from_str(harness).as_str();
        let pid = pid.to_string();
        if let Some(rec) = state
            .with_store(|s| s.alive_session_for_alias(Some(harness), "watch_pid", &pid))
            .ok()
            .flatten()
        {
            return Ok(rec);
        }
    }
    // Strict: exact anchors only. Never fall through to a sibling-binding scan.
    if scope == ResolveScope::Strict {
        anyhow::bail!(
            "must be run from within a tenex-edge agent session \
             (no --session, PTY session, harness id, or watch pid resolved a live session)"
        );
    }
    // 5. Scan: cwd-derived channel (or explicit group) + agent slug.
    //    `list_alive_sessions` is newest-first, so the first match is the latest.
    //    LIMITATION: with no exact anchor (for a native harness that has neither
    //    a PTY session nor a harness-native id), this picks the latest
    //    alive session for the agent in the channel — so it assumes a single live
    //    session per (agent, channel) there. Hosted launches never reach this
    //    tier because the PTY session anchor at step 2 is exact.
    let cwd = anchor
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let explicit_group = anchor.group.filter(|g| !g.is_empty());
    let work_root = explicit_group.is_none();
    let channel = explicit_group
        .map(|g| g.to_string())
        .unwrap_or_else(|| crate::workspace::resolve(&cwd).unwrap_or_default());
    let want_agent = anchor.agent.filter(|a| !a.is_empty());

    let pick = state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .find(|rec| {
                let scope_ok = rec.channel_h == channel
                    || (work_root && work_root_for(s, &rec.channel_h) == channel);
                let agent_ok = want_agent.map(|a| rec.agent_slug == a).unwrap_or(true);
                scope_ok && agent_ok
            })
    });
    if let Some(rec) = pick {
        return Ok(rec);
    }
    if let Some(agent) = want_agent {
        anyhow::bail!(
            "no active tenex-edge session for agent {agent:?} in channel {channel:?} (run session-start, or pass --session)"
        );
    }
    anyhow::bail!(
        "no active tenex-edge session for channel {channel:?} (run session-start, or pass --session)"
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
