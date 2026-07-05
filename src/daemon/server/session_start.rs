use super::*;

mod abort;
mod advisory;
mod channel_ready;
mod params;
mod stale;

use abort::abort_session_start;
use params::SessionStartParams;

/// The top-level project channel for a route scope.
fn work_root_for_scope(s: &Store, scope: &str) -> String {
    s.channel_project_root(scope)
        .ok()
        .flatten()
        .unwrap_or_else(|| scope.to_string())
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
        prog.emit(
            "identity",
            format!("loading local key for agent {}", p.agent),
        );
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
        // The relay must provision/confirm the named channel before the session can
        // be scoped to it. A degraded resolve used to fail OPEN to the work-root,
        // silently relocating the agent out of `launch --channel X` into the project
        // root; instead propagate the error so the launch command fails visibly. No
        // fabrication, no silent re-scope.
        Some(name) => super::resolve_channel(state, &work_root, &name, Some(&p.agent), true)
            .await
            .with_context(|| format!("resolving launch channel {name:?}"))?,
        None => work_root.clone(),
    };
    let rel_cwd = crate::project::rel_cwd(&cwd);
    let now = now_secs();
    if let Some(prog) = &progress {
        prog.emit(
            "project",
            format!("resolved project {project} from {}", cwd.display()),
        );
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
    tracing::debug!(agent = %p.agent, harness = %harness_str, channel = %project, "session_start hook received");
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
    // Resolve (or mint) the canonical session id WITHOUT writing the row yet. The
    // row is written further down, AFTER signer selection, so it is born carrying
    // this session's ordinal pubkey (never the base) — see the `upsert_session_row`
    // call below.
    let session_id =
        state.with_store(|s| s.resolve_or_mint_session_id(harness_str, ext_kind, &ext_id, now))?;
    if let Some(prog) = &progress {
        prog.emit(
            "session_registry",
            format!("session {session_id} registered"),
        );
    }

    // Record the secondary external-id aliases (pane/pid + any id not used as the
    // primary) and the project's absolute path on this machine. Reused pane/pid
    // slots repoint to this newest owner via ON CONFLICT.
    state.with_store(|s| {
        if let Some(pane) = &tmux_pane {
            s.put_alias(harness_str, "tmux_pane", pane, &session_id, now)
                .ok();
        }
        if let Some(pid) = p.watch_pid {
            s.put_alias(harness_str, "watch_pid", &pid.to_string(), &session_id, now)
                .ok();
        }
        if let Some(hs) = &harness_session_id {
            s.put_alias(harness_str, "harness_session", hs, &session_id, now)
                .ok();
        }
        if let Some(r) = &resume_id {
            s.put_alias(harness_str, "resume", r, &session_id, now).ok();
        }
        s.upsert_project_root(&project, &cwd.to_string_lossy(), now)
            .ok();
    });

    membership_cleanup::cleanup_dead_local_sessions(state);

    // A new logical session arriving on the SAME watched pid OR tmux pane (same
    // agent, same work root) means the harness restarted without a session-end.
    // Cancel its engine task, release its signer reservation, and mark it dead so
    // `who` doesn't show ghosts. (All sessions in this DB are this machine's.)
    {
        let new_work_root = room_parent
            .clone()
            .unwrap_or_else(|| state.with_store(|s| work_root_for_scope(s, &project)));
        stale::cancel_stale_sessions_on_restart(
            state,
            &session_id,
            &p.agent,
            p.watch_pid,
            tmux_pane.as_deref(),
            &new_work_root,
        );
    }

    // Select this session's ordinal identity, THEN write its row carrying that
    // ordinal pubkey. Ordering matters twice over: it runs AFTER stale-session
    // cancellation (so a superseded ordinal is freed and reusable), and BEFORE the
    // row is persisted / the re-assert early-return below (so the row is born with
    // the correct pubkey — and re-asserts refresh it to the SAME ordinal rather
    // than collapsing onto the base). `route_chat` keys on this `agent_pubkey`, so
    // a p-tagged mention reaches exactly this session, not every ordinal of the
    // agent. Membership admission for ordinals > 0 happens after channel-ready.
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
    let already_running = state.sessions.lock().unwrap().contains_key(&session_id);
    // If the engine is already running (re-assert from a duplicate spawn such as
    // the offline-agent-mention handler), preserve the live session's active
    // channel rather than stomping it with whatever TENEX_EDGE_CHANNEL the new
    // process was launched with. Without this guard, the duplicate's stale env
    // overwrites channel_h transiently AND permanently adds a spurious passive
    // join to session_channels (INSERT OR IGNORE never cleans it up), causing
    // the session to receive inbox messages from the wrong channel.
    let channel_for_upsert = if already_running {
        state
            .with_store(|s| s.get_session(&session_id).ok().flatten())
            .map(|r| r.channel_h)
            .unwrap_or_else(|| project.clone())
    } else {
        project.clone()
    };
    let effective_pane = tmux_pane
        .clone()
        .or_else(|| state.with_store(|s| session_pane(s, &session_id)));
    let needs_chat_replay = state.subs.lock().unwrap().covers_channel(&project);
    let base_pubkey = id.pubkey_hex();
    let request = advisory::request_fact(
        &session_id,
        &p.agent,
        harness_str,
        ext_kind,
        ext_id.clone(),
        &native_id,
        &work_root,
        &project,
        channel_for_upsert,
        &rel_cwd,
        room_parent.clone(),
        p.watch_pid,
        effective_pane.clone(),
        tmux_pane.is_some(),
        base_pubkey.clone(),
        signer.pubkey.clone(),
        signer.label.clone(),
        signer.ordinal,
        already_running,
        needs_chat_replay,
        now,
    );
    let observed = advisory::observed_command(&request);
    let plan = advisory::drive_request(state, request, &observed)?.plan;
    let reg = crate::state::RegisterSession {
        harness: plan.row.harness.clone(),
        external_id_kind: plan.row.external_id_kind.clone(),
        external_id: plan.row.external_id.clone(),
        agent_pubkey: plan.row.agent_pubkey.clone(),
        agent_slug: plan.row.agent_slug.clone(),
        channel_h: plan.row.channel_h.clone(),
        child_pid: plan.row.child_pid,
        transcript_path: None,
        resume_id: plan.row.resume_id.clone(),
        now: plan.row.now,
    };
    state.with_store(|s| s.upsert_session_row(&session_id, &reg))?;

    // Stamp the canonical session id onto the tmux session owning this pane so the
    // status-format `#(...)` can read it via `#{@te_session}`. When the
    // re-registration arrives without TMUX_PANE, fall back to the session's stored
    // pane alias so @te_session is never left stale. Best-effort, off the store lock.
    if let Some(pane) = &plan.tmux_pane {
        crate::tmux::set_pane_session_id(pane, &session_id, p.tmux_socket.as_deref());
    }
    // Ring on endpoint registration so delivery doesn't depend on the tmux TUI
    // running or on a later mention event.
    if plan.ring_doorbell {
        crate::tmux::ring_doorbells(state.clone());
    }

    // Idempotent re-start (session reassert): the engine task already runs.
    if plan.reassert {
        tracing::info!(
            agent = %p.agent,
            session = %session_id,
            channel = %project,
            "session re-assert: engine already running"
        );
        if let Some(prog) = &progress {
            prog.emit("session_start", "existing engine is already running");
        }
        advisory::record_started(
            state,
            &session_id,
            &plan.row.channel_h,
            &plan.row.agent_pubkey,
            plan.row.child_pid,
            now_secs(),
        );
        return Ok(serde_json::json!({
            "session_id": session_id,
        }));
    }

    // Make sure the channel exists + this agent is a member BEFORE the engine
    // starts publishing. Session start fails closed when relay readiness cannot
    // be verified; otherwise the engine could publish into phantom state.
    if let Some(prog) = &progress {
        prog.emit(
            "nip29",
            "checking NIP-29 channel state and membership on the relay",
        );
    }
    if let Some(check) = &plan.channel_ready {
        if let Err(e) = channel_ready::ensure_start_channel_ready(
            state,
            &check.channel_h,
            &check.work_root,
            check.room_parent.as_deref(),
            &check.base_pubkey,
            &session_id,
            progress.as_ref(),
        )
        .await
        {
            advisory::record_failed(state, &session_id, "channel_ready", &e, now_secs());
            return Err(e);
        }
    }

    // `signer` was selected above (before the row was written). Now that the
    // channel is ready, admit ordinals > 0 as NIP-29 members before routing use.
    if let Some(member_pubkey) = plan.admit_pubkey.as_deref() {
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
        if let Err(e) = admit_ordinal_signer(state, &project, member_pubkey).await {
            abort_session_start(state, &session_id);
            advisory::record_failed(state, &session_id, "admit_ordinal", &e, now_secs());
            return Err(e);
        }
    }

    if plan.notify_outbox {
        state.outbox_notify.notify_waiters();
    }

    if let Some(prog) = &progress {
        prog.emit(
            "subscription",
            "opening or refreshing project subscriptions",
        );
    }
    if plan.ensure_subscription {
        if let Err(e) = ensure_subscription(state, &project).await {
            tracing::warn!(channel = %project, error = %e, "subscription setup failed (session will continue)");
            if let Some(prog) = &progress {
                prog.emit(
                    "subscription",
                    format!("subscription setup failed but session will continue: {e:#}"),
                );
            }
        } else if let Some(prog) = &progress {
            prog.emit("subscription", "project subscription is active");
        }
    }

    if plan.replay_chat {
        replay_channel_chat(state, &project).await;
    }

    let Some(spawn) = &plan.spawn else {
        anyhow::bail!("session_start advisory plan did not include spawn intent");
    };
    let ep = engine_params_for(
        &state.cfg,
        &id,
        signer.instance(&p.agent, &id.pubkey_hex()),
        &spawn.session_id,
        &spawn.channel_h,
        &spawn.rel_cwd,
        spawn.watch_pid,
    );
    if let Some(prog) = &progress {
        prog.emit("engine", "starting session engine and initial publishers");
    }
    if let Err(e) = spawn_session(state, ep).await {
        advisory::record_failed(state, &session_id, "spawn_engine", &e, now_secs());
        return Err(e);
    }
    tracing::info!(
        agent = %p.agent,
        channel = %project,
        session = %session_id,
        harness = %harness_str,
        "session started"
    );
    if let Some(prog) = &progress {
        prog.emit("engine", "session engine started");
    }

    if plan.emit_tail {
        state.emit_tail(TailEvent::Sess {
            ts: now_secs(),
            project: project.clone(),
            agent: p.agent.clone(),
            session: session_id.clone(),
            state: "start".into(),
            rel_cwd: rel_cwd.clone(),
        });
    }

    advisory::record_started(
        state,
        &session_id,
        &plan.row.channel_h,
        &plan.row.agent_pubkey,
        plan.row.child_pid,
        now_secs(),
    );

    Ok(serde_json::json!({
        "session_id": session_id,
    }))
}
