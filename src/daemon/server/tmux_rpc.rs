use super::resolve_session;
use super::*;

/// The tmux pane id bound to a session, via the `tmux_pane` alias rows. Reused OS
/// panes repoint their alias to the newest owner, so the alias IS the endpoint.
fn tmux_pane_for_session(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    let aliases = state
        .with_store(|s| s.aliases_for_session(session_id))
        .ok()?;
    aliases
        .into_iter()
        .find(|a| a.external_id_kind == "tmux_pane")
        .map(|a| a.external_id)
}

/// The parent project a channel hangs under ('' parent = the channel itself).
fn work_root_for(state: &Arc<DaemonState>, scope: &str) -> String {
    match state.with_store(|s| s.channel_parent(scope)).ok().flatten() {
        Some(p) if !p.is_empty() => p,
        _ => scope.to_string(),
    }
}

// ── tmux_status ───────────────────────────────────────────────────────────────

pub(super) fn rpc_tmux_status(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let statuses = crate::tmux::list_endpoint_statuses(state);
    let arr: Vec<serde_json::Value> = statuses
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "session_id": s.session_id,
                "pane_id": s.pane_id,
                "pane_command": s.pane_command,
                "alive": s.alive,
                "registered_at": s.registered_at,
                "last_verified": s.last_verified,
            })
        })
        .collect();
    Ok(serde_json::json!({ "endpoints": arr }))
}

// ── tmux_send (manual pending-message injection) ──────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxSendParams {
    session: String,
}

pub(super) async fn rpc_tmux_send(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxSendParams =
        serde_json::from_value(params.clone()).context("parsing tmux_send params")?;

    // Resolve the session (supports prefix matching via resolve_session fallback).
    let rec = resolve_session(state, Some(&p.session), None, None, None, None)
        .with_context(|| format!("no session matching {:?}", p.session))?;

    let pane_id = match tmux_pane_for_session(state, &rec.session_id) {
        Some(p) => p,
        None => {
            return Ok(serde_json::json!({
                "injected": false,
                "reason": "no tmux endpoint registered for this session"
            }));
        }
    };

    if crate::tmux::pane_alive_pub(&pane_id).is_none() {
        return Ok(serde_json::json!({
            "injected": false,
            "reason": format!("pane {pane_id} is gone")
        }));
    }

    let injected = crate::tmux::inject_pending_messages_pub(state, &rec, &pane_id).await?;

    if injected {
        Ok(serde_json::json!({ "injected": true, "pane_id": pane_id }))
    } else {
        Ok(serde_json::json!({
            "injected": false,
            "pane_id": pane_id,
            "reason": "no unread messages for this session"
        }))
    }
}

// ── tmux_spawn ────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxSpawnParams {
    agent: String,
    project: String,
    #[serde(default)]
    command: Vec<String>,
    /// Override the entire base command, replacing what would be resolved from
    /// the agent file. When non-empty, `command` (extra args) is still appended.
    /// Forwarded from `tenex-edge launch -c <string>`.
    #[serde(default)]
    base_command: Vec<String>,
    /// The client's cwd, forwarded so the daemon spawns the agent in the
    /// directory the user actually invoked `tenex-edge launch` from — NOT the
    /// daemon's own cwd (which is sticky and never matches the client's). When
    /// present, this wins over `project_roots` lookup and also updates the
    /// `project_roots` row so subsequent spawns without `cwd` still find it.
    #[serde(default)]
    cwd: Option<String>,
    /// The RESOLVED opaque channel id to scope the spawned session into (the CLI
    /// launch path already converted any `--channel <name>` to its id via
    /// `channels_resolve`, so no literal-name path reaches here). Sets
    /// `TENEX_EDGE_CHANNEL` in the pane env so the session publishes into this
    /// group instead of its per-session room. The daemon's tenexPrivateKey adds
    /// the agent as a member on session-start.
    #[serde(default)]
    channel: Option<String>,
}

pub(super) async fn rpc_tmux_spawn(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxSpawnParams =
        serde_json::from_value(params.clone()).context("parsing tmux_spawn params")?;
    let client_cwd = p.cwd.as_deref().map(std::path::Path::new);
    let base_override = if p.base_command.is_empty() {
        None
    } else {
        Some(p.base_command)
    };
    let group = p.channel.as_deref();

    // Proactively provision the channel/project BEFORE opening the pane so the
    // relay already has the group and the agent as a member when the first
    // session-start event arrives. Best-effort with an 8-second cap — a slow
    // relay must never block the spawn. Session-start repeats this idempotently.
    provision_before_spawn(state, &p.agent, &p.project, group).await;

    let pane_id = crate::tmux::spawn_agent(
        state,
        &p.agent,
        &p.project,
        p.command,
        base_override,
        group,
        client_cwd,
        None,
    )
    .await?;
    Ok(serde_json::json!({ "pane_id": pane_id, "agent": p.agent, "project": p.project }))
}

/// Resolve the agent's durable pubkey and call `ensure_channel_ready` for the
/// launch scope (the channel if given, else the project root) before the pane is
/// opened.
///
/// If the same agent slug already has a live session in the scope, logs a note
/// about the concurrent launch — ordinal-keyed second-instance pubkeys are a
/// future extension; for now both instances share the same durable key.
async fn provision_before_spawn(
    state: &Arc<DaemonState>,
    slug: &str,
    project: &str,
    channel: Option<&str>,
) {
    let edge = crate::config::edge_home();
    let id = match crate::identity::load_or_create(&edge, slug, crate::util::now_secs()) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("[daemon] tmux_spawn: could not resolve identity for {slug}: {e:#}");
            return;
        }
    };
    let pubkey = id.pubkey_hex();

    // Detect concurrent instances of the same agent in this scope.
    let scope = channel.filter(|g| !g.is_empty()).unwrap_or(project);
    let already_live = state
        .with_store(|s| s.list_alive_sessions())
        .unwrap_or_default()
        .iter()
        .any(|r| r.agent_slug == slug && r.channel_h == scope);
    if already_live {
        eprintln!(
            "[daemon] tmux_spawn: {slug} already has a live session in {scope}; \
             launching concurrent instance (shared pubkey {}; ordinal keys are a future extension)",
            crate::util::pubkey_short(&pubkey),
        );
    }

    let timeout = std::time::Duration::from_secs(8);
    // One primitive provisions every channel: a top-level project is the ROOT
    // channel (parent_hint None); an explicit channel is a subgroup under the
    // project (parent_hint = project). `ensure_channel_ready` ensures existence +
    // admin invariants + membership either way, so the session-start that follows
    // finds the scope ready (with a valid parent when a per-session room is minted).
    let parent_hint = channel.filter(|g| !g.is_empty()).map(|_| project);
    eprintln!(
        "[daemon] tmux_spawn: pre-provisioning channel {scope} for {slug} ({})",
        crate::util::pubkey_short(&pubkey),
    );
    let ctx = crate::fabric::nip29::readiness::ChannelCtx {
        channel: scope,
        expect_member: &pubkey,
        parent_hint,
    };
    if tokio::time::timeout(timeout, state.provider.ensure_channel_ready(ctx))
        .await
        .is_err()
    {
        eprintln!("[daemon] tmux_spawn: channel provisioning timed out for {scope}; proceeding");
    }
}

// ── tmux_attach ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxAttachParams {
    session: String,
}

pub(super) fn rpc_tmux_attach(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxAttachParams =
        serde_json::from_value(params.clone()).context("parsing tmux_attach params")?;
    let rec = resolve_session(state, Some(&p.session), None, None, None, None)
        .with_context(|| format!("no session matching {:?}", p.session))?;
    match tmux_pane_for_session(state, &rec.session_id) {
        Some(pane) => Ok(serde_json::json!({ "pane_id": pane, "session_id": rec.session_id })),
        None => Ok(serde_json::json!({
            "error": "no tmux endpoint registered for this session"
        })),
    }
}

// ── tmux_resume ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TmuxResumeParams {
    session: String,
}

/// The harness-native resume token for a session, or `None` if we can't resume it.
///
/// Priority: an explicitly-stored `resume_id` (opencode forwards its `ses_*`),
/// else the `session_id` itself — for claude/codex we ADOPT their native id as
/// the session id, so it IS the resume token. Only our own synthetic `te-*` ids
/// (generated when a host supplies none, e.g. opencode without a captured id)
/// are not resume tokens, so those fall through to `None`.
fn resume_token_for(rec: &crate::state::Session) -> Option<String> {
    if !rec.resume_id.is_empty() {
        return Some(rec.resume_id.clone());
    }
    if rec.session_id.starts_with("te-") {
        return None;
    }
    Some(rec.session_id.clone())
}

/// Resume a (typically dead) session by replaying its harness with the captured
/// native resume token. Spawns a new tmux window and returns its pane id.
pub(super) async fn rpc_tmux_resume(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TmuxResumeParams =
        serde_json::from_value(params.clone()).context("parsing tmux_resume params")?;

    // Resolve including dead sessions: exact id (get_session) first, then a
    // session-id prefix, then the codename the TUI displays (e.g. `bravo4217`) —
    // resolve_session only matches alive rows by cwd/agent.
    let rec = match state
        .with_store(|s| s.get_session(&p.session))
        .ok()
        .flatten()
    {
        Some(r) => r,
        None => state
            .with_store(|s| s.find_session_by_prefix(&p.session))
            .ok()
            .flatten()
            .or_else(|| resume_by_codename(state, &p.session))
            .with_context(|| format!("no session matching {:?}", p.session))?,
    };

    // All sessions in the `sessions` table are hosted by THIS machine, so a
    // resolved row is always locally resumable — no cross-host guard needed.

    let resume_id = match resume_token_for(&rec) {
        Some(id) => id,
        None => {
            return Ok(serde_json::json!({
                "error": "session has no resume token (not resumable)"
            }));
        }
    };

    // Re-scope the resumed session to the SAME channel it had when it exited.
    // Passing the scope as the group override sets `TENEX_EDGE_CHANNEL` so the
    // resumed session publishes into the right channel without restarting.
    let scope = rec.channel_h.clone();
    match crate::tmux::resume_agent(state, &rec.agent_slug, &scope, &resume_id).await {
        Ok(pane_id) => Ok(serde_json::json!({
            "pane_id": pane_id,
            "session_id": rec.session_id,
            "agent": rec.agent_slug,
        })),
        Err(e) => Ok(serde_json::json!({ "error": format!("{e:#}") })),
    }
}

/// Resolve a session by the codename the TUI displays (e.g. `bravo4217`), scanning
/// recent local sessions (including dead rows) so a user can copy `[session
/// bravo4217]` straight into `tmux resume`. Case-insensitive; first match wins.
fn resume_by_codename(state: &Arc<DaemonState>, target: &str) -> Option<crate::state::Session> {
    let want = target.to_lowercase();
    state
        .with_store(|s| s.list_resumable_sessions(200))
        .ok()
        .unwrap_or_default()
        .into_iter()
        .find(|rec| crate::util::session_codename(&rec.session_id).to_lowercase() == want)
}

// ── tmux_resumable ────────────────────────────────────────────────────────────

/// List recent local sessions that are resumable but NOT in a live tmux pane.
/// "Dead" rows only — sessions still alive on the fabric appear in the live list
/// and are resumable from there via `[r]`; this section is the longer tail of
/// sessions that have exited entirely. Newest first.
pub(super) fn rpc_tmux_resumable(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    const LIMIT: u32 = 60;
    let candidates = state
        .with_store(|s| s.list_resumable_sessions(LIMIT))
        .unwrap_or_default();

    let arr: Vec<serde_json::Value> = candidates
        .into_iter()
        .filter_map(|rec| {
            // Must have a usable resume token (claude/codex: the session id;
            // opencode: a captured ses_*; our synthetic te-* ids: not resumable).
            resume_token_for(&rec)?;
            // Alive sessions are shown in the live list (resume them with [r]
            // there); keep this section to fully-exited ones to avoid dupes.
            if rec.alive {
                return None;
            }
            // Skip sessions with a live pane — those are attachable, not resume
            // candidates. A missing/dead alias means the harness is gone.
            let live_pane = tmux_pane_for_session(state, &rec.session_id)
                .is_some_and(|p| crate::tmux::pane_alive_pub(&p).is_some());
            if live_pane {
                return None;
            }
            let work_root = work_root_for(state, &rec.channel_h);
            Some(serde_json::json!({
                "session_id": rec.session_id,
                "slug": rec.agent_slug,
                "project": rec.channel_h,
                "work_root": work_root,
                "rel_cwd": "",
                "alive": rec.alive,
                "created_at": rec.created_at,
                "title": rec.title,
            }))
        })
        .collect();

    Ok(serde_json::json!({ "resumable": arr }))
}
