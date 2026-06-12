use super::lifecycle::{cancel_session, engine_params_for, ensure_subscription, spawn_session};
use super::*;

// ── session resolution (daemon-side, identical to the old CLI) ───────────────

/// Resolve the caller's session like the pre-daemon CLI did, but AGENT-SCOPED:
/// explicit id → the `env_session` the host exported → most-recent alive session
/// for the project of `cwd` **belonging to the invoking agent** (`agent`, from
/// `$TENEX_EDGE_AGENT`). The agent-scoped fallback is the fix for the bug where a
/// `claude` send-message was signed/recorded as `opencode` merely because an
/// opencode session was the latest-active in the project. If `agent` is unknown
/// (older clients that don't thread it), fall back to the agent-agnostic
/// latest-alive lookup to preserve prior behavior.
pub(super) fn resolve_session(
    state: &Arc<DaemonState>,
    explicit: Option<&str>,
    env_session: Option<&str>,
    cwd: Option<&str>,
    agent: Option<&str>,
) -> Result<crate::state::SessionRecord> {
    if let Some(id) = explicit.filter(|s| !s.is_empty()) {
        return state
            .with_store(|s| s.get_session(id))
            .with_context(|| format!("unknown session {id}"))?
            .with_context(|| format!("unknown session {id}"));
    }
    if let Some(id) = env_session.filter(|s| !s.is_empty()) {
        if let Some(rec) = state.with_store(|s| s.get_session(id)).ok().flatten() {
            return Ok(rec);
        }
    }
    let cwd = cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = crate::project::resolve(&cwd);
    if let Some(agent) = agent.filter(|a| !a.is_empty()) {
        if let Some(rec) =
            state.with_store(|s| s.latest_alive_session_for_agent_in_project(agent, &project))?
        {
            return Ok(rec);
        }
        anyhow::bail!(
            "no active tenex-edge session for agent {agent:?} in project {project:?} (run session-start, or pass --session)"
        );
    }
    state
        .with_store(|s| s.latest_alive_session_for_project(&project))?
        .with_context(|| {
            format!("no active tenex-edge session for project {project:?} (run session-start, or pass --session)")
        })
}

// ── session_start / session_end ──────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct SessionStartParams {
    agent: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    watch_pid: Option<i32>,
}

pub(super) async fn rpc_session_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionStartParams =
        serde_json::from_value(params.clone()).context("parsing session_start params")?;
    let edge = config::edge_home();
    config::ensure_dir(&edge)?;
    let id = identity::load_or_create(&edge, &p.agent, now_secs())?;
    let _ = crate::acl::allow(&id.pubkey_hex(), &p.agent); // own fleet auto-trusted
    let cwd = p
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = crate::project::resolve(&cwd);
    let rel_cwd = crate::project::rel_cwd(&cwd);
    let session_id = p.session_id.unwrap_or_else(gen_session_id);
    if let Some(watch_pid) = p.watch_pid {
        let stale_ids: Vec<String> = state
            .with_store(|s| s.list_alive_sessions().unwrap_or_default())
            .into_iter()
            .filter(|rec| {
                rec.session_id != session_id
                    && rec.agent_slug == p.agent
                    && rec.project == project
                    && rec.host == state.host
                    && rec.watch_pid == Some(watch_pid)
            })
            .map(|rec| rec.session_id)
            .collect();
        for old_id in stale_ids {
            cancel_session(state, &old_id);
            state.with_store(|s| {
                s.mark_session_dead(&old_id).ok();
            });
        }
    }

    state.with_store(|s| {
        s.upsert_session(&crate::state::SessionRecord {
            session_id: session_id.clone(),
            agent_slug: p.agent.clone(),
            agent_pubkey: id.pubkey_hex(),
            project: project.clone(),
            host: state.host.clone(),
            child_pid: None,
            watch_pid: p.watch_pid,
            created_at: now_secs(),
            alive: true,
            rel_cwd: rel_cwd.clone(),
        })
        .ok();
        s.touch_session(&session_id, now_secs()).ok();
    });
    if state.sessions.lock().unwrap().contains_key(&session_id) {
        return Ok(serde_json::json!({ "session_id": session_id }));
    }

    // Make sure the project's NIP-29 group exists and this agent is a member
    // BEFORE the engine starts publishing, so its presence lands in a group it
    // already belongs to. Best-effort: never block a session from starting.
    ensure_group_and_membership(state, &project, &id.pubkey_hex()).await;

    let ep = engine_params_for(
        &state.cfg,
        &id,
        &p.agent,
        &session_id,
        &project,
        &rel_cwd,
        p.watch_pid,
    );
    spawn_session(state, ep).await?;

    Ok(serde_json::json!({ "session_id": session_id }))
}

/// Ensure the operator owns a closed NIP-29 group for `project` and that
/// `agent_pubkey` is a member — all signed by the operator's `userNsec`. Every
/// step is best-effort: a missing `userNsec` or a flaky relay must NOT prevent a
/// session from starting (unlike `rpc_user_prompt`, which bails). The relay rules
/// here are validated by `tests/nip29_probe.rs`.
pub(super) async fn ensure_group_and_membership(
    state: &Arc<DaemonState>,
    project: &str,
    agent_pubkey: &str,
) {
    use nostr_sdk::prelude::Keys;
    let nsec = match &state.cfg.user_nsec {
        Some(n) => n.clone(),
        None => {
            // No operator key → can't manage groups. Sessions still run; the
            // relay just won't enforce membership for this project.
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!(
                    "[daemon] userNsec unset; skipping NIP-29 group management for {project}"
                );
            }
            return;
        }
    };
    let user_keys = match Keys::parse(&nsec) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("[daemon] userNsec parse failed; skipping group management: {e}");
            return;
        }
    };

    // Publish a group-management event, returning whether the relay now reflects
    // it. `publish_signed_checked` errors on relay rejection (rate-limited,
    // blocked, …); we treat an "already exists" rejection as success so a daemon
    // restart over an already-created group still converges. Anything else
    // (rate-limit, network, not-admin) is a genuine failure: we must NOT record
    // success, or a transient blip would permanently poison the cache (mark a
    // nonexistent group "owned"/agent "member" and never retry → presence writes
    // blocked forever). On failure we leave the cache untouched and the next
    // session_start retries.
    let publish = |builder, label: &'static str| {
        let transport = state.transport.clone();
        let keys = user_keys.clone();
        async move {
            match transport.publish_signed_checked(builder, &keys).await {
                Ok(()) => true,
                Err(e) => {
                    let benign = {
                        let s = e.to_string();
                        s.contains("already exists") || s.contains("duplicate")
                    };
                    if !benign && std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                        eprintln!("[daemon] NIP-29 {label} publish failed (will retry next session): {e:#}");
                    }
                    benign
                }
            }
        }
    };

    // 1. Create + lock the group the first time we touch this project. Only mark
    //    it owned if BOTH create and the closed-lock actually landed — otherwise
    //    we'd leave an open/nonexistent group cached as owned and never re-lock.
    if !state.with_store(|s| s.is_group_owned(project).unwrap_or(false)) {
        let created = match crate::codec::kind1::group_create(project) {
            Ok(b) => publish(b, "9007 create-group").await,
            Err(_) => false,
        };
        let locked = if created {
            match crate::codec::kind1::group_lock_closed(project) {
                Ok(b) => publish(b, "9002 lock-closed").await,
                Err(_) => false,
            }
        } else {
            false
        };
        if created && locked {
            state.with_store(|s| {
                s.mark_group_owned(project, now_secs()).ok();
            });
        }
    }

    // 2. Add this agent as a member if it isn't one already — but only cache the
    //    membership once the relay accepts the put-user, so a failed add retries.
    if !state.with_store(|s| s.is_group_member(project, agent_pubkey).unwrap_or(false)) {
        let added = match crate::codec::kind1::group_put_user(project, agent_pubkey) {
            Ok(b) => publish(b, "9000 put-user").await,
            Err(_) => false,
        };
        if added {
            state.with_store(|s| {
                s.upsert_group_member(project, agent_pubkey, "member", now_secs())
                    .ok();
            });
        }
    }

    // Keep the relay-authored group state (39000/39001/39002) subscribed so the
    // membership cache stays current — "check which groups we own at all times".
    if let Err(e) = ensure_subscription(state, project).await {
        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
            eprintln!("[daemon] ensure_subscription({project}) failed: {e:#}");
        }
    }
}

#[derive(serde::Deserialize)]
struct SessionEndParams {
    session: String,
}

pub(super) fn rpc_session_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionEndParams =
        serde_json::from_value(params.clone()).context("parsing session_end params")?;
    let existed = state.with_store(|s| s.get_session(&p.session).ok().flatten().is_some());
    if existed {
        cancel_session(state, &p.session);
        state.with_store(|s| {
            s.mark_session_dead(&p.session).ok();
        });
    }
    Ok(serde_json::json!({ "ended": existed }))
}

fn gen_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("te-{nanos:x}-{}", std::process::id())
}

// ── send_message ─────────────────────────────────────────────────────────────

// ── session lifecycle ─────────────────────────────────────────────────────────
