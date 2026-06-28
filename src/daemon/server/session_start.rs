use super::*;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct SessionStartParams {
    agent: String,
    /// The harness-native external session id. Hooks send it as
    /// `harness_session_id`; the legacy/CLI path sends `session_id`. Either is
    /// accepted — it is ONLY a locator for `session_aliases`, never the identity.
    #[serde(default, alias = "harness_session_id")]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    watch_pid: Option<i32>,
    /// Stable tmux pane id from $TMUX_PANE (e.g. "%5"). Present only when the hook
    /// fires inside a tmux session.
    #[serde(default)]
    tmux_pane: Option<String>,
    /// Value of $TMUX (socket path, session id, pane id).
    #[serde(default)]
    tmux_socket: Option<String>,
    /// Harness-native resume token, supplied explicitly by programmatic hosts
    /// (opencode forwards its `ses_*` id here). For claude-code/codex this is
    /// absent — their adopted `session_id` IS the resume token.
    #[serde(default)]
    resume_id: Option<String>,
    /// Which harness produced this hook (`claude-code`|`codex`|`opencode`). When
    /// absent, inferred from the id/resume shape for alias namespacing.
    #[serde(default)]
    harness: Option<String>,
    /// NIP-29 channel (`h`) this pane was spawned into (from `TENEX_EDGE_CHANNEL`).
    /// When present the session is scoped to this channel instead of the
    /// working-directory project. The working directory remains the parent repo.
    #[serde(default)]
    channel: Option<String>,
    /// Exact ordinal to allocate for this session (issue #47), forwarded from
    /// `TENEX_EDGE_ORDINAL` by a spawn-on-mention that targeted a specific
    /// `smithN`. When present the signer honors it instead of lowest-free.
    #[serde(default)]
    preferred_ordinal: Option<u32>,
}

/// The top-level project channel for a route scope: a channel's non-empty parent,
/// else the scope itself (a root channel is its own work root).
fn work_root_for_scope(s: &Store, scope: &str) -> String {
    match s.channel_parent(scope).ok().flatten() {
        Some(p) if !p.is_empty() => p,
        _ => scope.to_string(),
    }
}

/// The tmux pane id currently bound to a session, via its `tmux_pane` alias.
fn session_pane(s: &Store, session_id: &str) -> Option<String> {
    s.aliases_for_session(session_id)
        .ok()?
        .into_iter()
        .find(|a| a.external_id_kind == "tmux_pane")
        .map(|a| a.external_id)
}

pub(in crate::daemon::server) async fn rpc_session_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    progress: Option<InitProgress>,
) -> Result<serde_json::Value> {
    if let Some(prog) = &progress {
        prog.emit("session_start", "parsing hook payload");
    }
    let p: SessionStartParams =
        serde_json::from_value(params.clone()).context("parsing session_start params")?;
    let edge = config::edge_home();
    config::ensure_dir(&edge)?;
    if let Some(prog) = &progress {
        prog.emit("identity", format!("loading local key for agent {}", p.agent));
    }
    let id = identity::load_or_create(&edge, &p.agent, now_secs())?;
    let cwd = p
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    // The working-directory project (the repo this harness runs in).
    let work_root = crate::project::resolve(&cwd).unwrap_or_default();
    // The channel this session belongs to: the channel named by TENEX_EDGE_CHANNEL
    // for a task room, else the working-directory project. The daemon is a direct
    // entry point (e2e drives `hook --type session-start` with a NAME in
    // TENEX_EDGE_CHANNEL), so resolve the NAME→opaque id through the ONE shared
    // resolver here. This is the single load-bearing conversion that kills the
    // name-vs-id double-create: every downstream consumer shares one id, and the
    // literal-name subgroup is never minted.
    let mut project = match p.channel.clone().filter(|g| !g.is_empty()) {
        Some(name) => {
            super::resolve_channel(state, &work_root, &name, Some(&p.agent), true).await?
        }
        None => work_root.clone(),
    };
    let rel_cwd = crate::project::rel_cwd(&cwd);
    let now = now_secs();
    if let Some(prog) = &progress {
        prog.emit("project", format!("resolved project {project} from {}", cwd.display()));
    }

    // Normalize the hook's identity inputs. claude-code/codex adopt their native
    // `session_id` (doubles as the resume token); opencode supplies no
    // `session_id` and forwards its `ses_*` resume token instead. The harness
    // label is explicit when sent, else inferred from that shape.
    let harness_session_id = p.session_id.clone().filter(|s| !s.is_empty());
    let resume_id = p.resume_id.clone().filter(|s| !s.is_empty());
    let harness = p
        .harness
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(Harness::from_str)
        .unwrap_or_else(|| {
            if resume_id.is_some() {
                Harness::Opencode
            } else if harness_session_id.is_some() {
                Harness::ClaudeCode
            } else {
                Harness::Unknown
            }
        });
    let harness_str = harness.as_str();
    let tmux_pane = p.tmux_pane.clone().filter(|s| !s.is_empty());
    // The harness-native id to bind for resume: opencode `ses_*`, else claude/codex
    // native id.
    let native_id = resume_id
        .clone()
        .or_else(|| harness_session_id.clone())
        .unwrap_or_default();

    // Per-session rooms (issue #6), gated by `perSessionRooms` (default off). A
    // human-initiated session (no TENEX_EDGE_CHANNEL override) lives in its own
    // minted subgroup of the work-root when enabled, else the bare project. The
    // room id is derived from a stable per-session anchor so a resumed session
    // rejoins the SAME room. `room_parent` is `Some(parent)` exactly when we routed
    // into a freshly-minted room.
    let pid_anchor = p.watch_pid.map(|pid| format!("pid-{pid}"));
    let room_parent: Option<String> = {
        let anchor = harness_session_id
            .as_deref()
            .or(resume_id.as_deref())
            .or(pid_anchor.as_deref());
        match (
            crate::session::decide_session_room(
                p.channel.as_deref(),
                &work_root,
                state.cfg.per_session_rooms,
            ),
            anchor,
        ) {
            (crate::session::RoomDecision::Mint { parent }, Some(anchor)) => {
                project = crate::util::session_room_id(anchor);
                Some(parent)
            }
            _ => None,
        }
    };

    if let Some(prog) = &progress {
        prog.emit("session_registry", "registering or reasserting session");
    }
    // Canonical identity: the daemon MINTS a stable session id; the harness id /
    // resume token / pane / pid become rows in `session_aliases`. The primary
    // external id selects which harness-native locator keys the canonical session;
    // claude/codex use their native id, opencode its resume token, else the pid.
    let (ext_kind, ext_id) = if let Some(hs) = &harness_session_id {
        ("harness_session", hs.clone())
    } else if let Some(r) = &resume_id {
        ("resume", r.clone())
    } else if let Some(pid) = p.watch_pid {
        ("watch_pid", pid.to_string())
    } else {
        ("harness_session", String::new())
    };
    let reg = crate::state::RegisterSession {
        harness: harness_str.to_string(),
        external_id_kind: ext_kind.to_string(),
        external_id: ext_id,
        agent_pubkey: id.pubkey_hex(),
        agent_slug: p.agent.clone(),
        channel_h: project.clone(),
        child_pid: p.watch_pid,
        transcript_path: None,
        resume_id: native_id.clone(),
        now,
    };
    let session_id = state.with_store(|s| s.register_session(&reg))?;
    if let Some(prog) = &progress {
        prog.emit(
            "session_registry",
            format!("session {} registered", crate::util::session_codename(&session_id)),
        );
    }

    // Record the secondary external-id aliases (pane/pid + any id not used as the
    // primary) and the project's absolute path on this machine. Reused pane/pid
    // slots repoint to this newest owner via ON CONFLICT.
    state.with_store(|s| {
        if let Some(pane) = &tmux_pane {
            s.put_alias(harness_str, "tmux_pane", pane, &session_id, now).ok();
        }
        if let Some(pid) = p.watch_pid {
            s.put_alias(harness_str, "watch_pid", &pid.to_string(), &session_id, now).ok();
        }
        if let Some(hs) = &harness_session_id {
            s.put_alias(harness_str, "harness_session", hs, &session_id, now).ok();
        }
        if let Some(r) = &resume_id {
            s.put_alias(harness_str, "resume", r, &session_id, now).ok();
        }
        s.upsert_project_root(&project, &cwd.to_string_lossy(), now).ok();
    });

    // A new logical session arriving on the SAME watched pid OR tmux pane (same
    // agent, same work root) means the harness restarted without a session-end.
    // Cancel its engine task, release its signer reservation, and mark it dead so
    // `who` doesn't show ghosts. (All sessions in this DB are this machine's.)
    {
        let new_work_root = room_parent
            .clone()
            .unwrap_or_else(|| state.with_store(|s| work_root_for_scope(s, &project)));
        let alive = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
        let mut stale_ids: Vec<String> = Vec::new();
        for rec in &alive {
            if rec.session_id == session_id || rec.agent_slug != p.agent {
                continue;
            }
            let same_work_root =
                state.with_store(|s| work_root_for_scope(s, &rec.channel_h)) == new_work_root;
            if !same_work_root {
                continue;
            }
            let same_pid = p.watch_pid.is_some() && rec.child_pid == p.watch_pid;
            let same_pane = tmux_pane.as_deref().is_some_and(|pane| {
                state.with_store(|s| session_pane(s, &rec.session_id)).as_deref() == Some(pane)
            });
            if same_pid || same_pane {
                state.release_session_signer(&rec.session_id);
                stale_ids.push(rec.session_id.clone());
            }
        }
        for old_id in stale_ids {
            cancel_session(state, &old_id);
            state.with_store(|s| {
                s.mark_dead(&old_id).ok();
                s.mark_identity_dead_for_session(&old_id).ok();
            });
        }
    }

    // Stamp the canonical session id onto the tmux session owning this pane so the
    // status-format `#(...)` can read it via `#{@te_session}`. When the
    // re-registration arrives without TMUX_PANE, fall back to the session's stored
    // pane alias so @te_session is never left stale. Best-effort, off the store lock.
    let effective_pane = tmux_pane
        .clone()
        .or_else(|| state.with_store(|s| session_pane(s, &session_id)));
    if let Some(pane) = &effective_pane {
        crate::tmux::set_pane_session_id(pane, &session_id, p.tmux_socket.as_deref());
    }
    // Ring on endpoint registration so delivery doesn't depend on the tmux TUI
    // running or on a later mention event.
    if tmux_pane.is_some() {
        crate::tmux::ring_doorbells(state.clone());
    }

    // Idempotent re-start (session reassert): the engine task already runs.
    if state.sessions.lock().unwrap().contains_key(&session_id) {
        if let Some(prog) = &progress {
            prog.emit("session_start", "existing engine is already running");
        }
        return Ok(serde_json::json!({
            "session_id": session_id,
            "codename": crate::util::session_codename(&session_id),
        }));
    }

    // Make sure the channel exists + this agent is a member BEFORE the engine
    // starts publishing. Best-effort: never block a session from starting.
    if let Some(prog) = &progress {
        prog.emit("nip29", "checking NIP-29 channel state and membership on the relay");
    }
    if let Some(parent) = &room_parent {
        // Human-initiated session: mint its per-session room under the work-root.
        // Mark the channel in the LOCAL cache SYNCHRONOUSLY so room gates recognize
        // it immediately, then do all relay-dependent work in the BACKGROUND
        // (fail-open) so a slow relay never delays the engine or the first prompt.
        if let Some(prog) = &progress {
            prog.emit("nip29", format!("minting per-session room {project}"));
        }
        state.with_store(|s| {
            s.upsert_channel(&project, &project, "", parent, now_secs()).ok();
        });
        let st = state.clone();
        let room = project.clone();
        let par = parent.clone();
        let name = project.clone();
        let agent_pubkey = id.pubkey_hex();
        tokio::spawn(async move {
            ensure_session_room(&st, &room, &name, &par, &agent_pubkey).await;
        });
    } else {
        // Project / orchestration sessions: ensure the channel exists + the agent
        // is a member, bounded so a hung relay can't block session start. A
        // top-level project is the ROOT channel (parent_hint None); an explicit
        // channel scope (project != work_root) is a subgroup whose parent project
        // is ensured first (parent_hint = work_root).
        let parent_hint = if project != work_root && !work_root.is_empty() {
            Some(work_root.clone())
        } else {
            None
        };
        // Stamp the parent relationship immediately so `work_root_for_scope`
        // resolves without waiting for the relay's kind:39000 with the parent tag.
        // Preserve any existing human `name`/`about` (the resolver already stamped
        // them for a named channel) — only fall back to the id when truly unknown.
        if let Some(parent) = &parent_hint {
            state
                .with_store(|s| {
                    let existing = s.get_channel(&project).ok().flatten();
                    let name = existing
                        .as_ref()
                        .map(|c| c.name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| project.clone());
                    let about = existing.map(|c| c.about).unwrap_or_default();
                    s.upsert_channel(&project, &name, &about, parent, now_secs())
                })
                .ok();
        }
        let open = async {
            let ctx = crate::fabric::nip29::readiness::ChannelCtx {
                channel: &project,
                expect_member: &id.pubkey_hex(),
                parent_hint: parent_hint.as_deref(),
            };
            state.provider.ensure_channel_ready(ctx).await;
        };
        if tokio::time::timeout(std::time::Duration::from_secs(8), open)
            .await
            .is_err()
            && std::env::var("TENEX_EDGE_DEBUG").is_ok()
        {
            eprintln!("[daemon] ensure_channel_ready({project}) timed out (best-effort)");
        }
    }

    let signer = select_session_signer(
        state,
        &session_id,
        &id.keys,
        &id.pubkey_hex(),
        &p.agent,
        &project,
        &native_id,
        p.preferred_ordinal,
    )?;
    if let Some(member_pubkey) = signer.member_pubkey_to_admit() {
        if let Some(prog) = &progress {
            prog.emit(
                "nip29",
                format!(
                    "admitting ordinal {} signer {} before routing use",
                    signer.ordinal,
                    pubkey_short(member_pubkey)
                ),
            );
        }
        if let Err(e) = admit_transient_signer(state, &project, member_pubkey).await {
            state.release_session_signer(&session_id);
            state.with_store(|s| s.mark_identity_dead_for_session(&session_id).ok());
            return Err(e);
        }
    }

    // Nudge the drainer now that signer selection/admission is complete: the
    // pending first kind:30315 must be signed by the selected identity.
    state.outbox_notify.notify_waiters();

    if let Some(prog) = &progress {
        prog.emit("subscription", "opening or refreshing project subscriptions");
    }
    // Was the channel already subscribed before this session? If so, a mention may
    // have been published to it BEFORE this session existed (spawn-on-mention) and
    // the live materialize path never delivered it. We replay below once alive. A
    // freshly-subscribed channel needs no replay: opening the new REQ streams its
    // backlog to this (already-alive) session.
    let needs_chat_replay = state.subscriptions.lock().unwrap().covers_channel(&project);
    if let Err(e) = ensure_subscription(state, &project).await {
        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
            eprintln!("[daemon] ensure_subscription({project}) failed: {e:#}");
        }
        if let Some(prog) = &progress {
            prog.emit(
                "subscription",
                format!("subscription setup failed but session will continue: {e:#}"),
            );
        }
    } else if let Some(prog) = &progress {
        prog.emit("subscription", "project subscription is active");
    }

    if needs_chat_replay {
        replay_channel_chat(state, &project).await;
    }

    let ep = engine_params_for(
        &state.cfg,
        &id,
        &p.agent,
        &session_id,
        &project,
        &rel_cwd,
        p.watch_pid,
        signer.session_keys(),
    );
    if let Some(prog) = &progress {
        prog.emit("engine", "starting session engine and initial publishers");
    }
    spawn_session(state, ep).await?;
    if let Some(prog) = &progress {
        prog.emit("engine", "session engine started");
    }

    state.emit_tail(TailEvent::Sess {
        ts: now_secs(),
        project: project.clone(),
        agent: p.agent.clone(),
        session: session_id.clone(),
        state: "start".into(),
        rel_cwd: rel_cwd.clone(),
    });

    Ok(serde_json::json!({
        "session_id": session_id,
        "codename": crate::util::session_codename(&session_id),
    }))
}
