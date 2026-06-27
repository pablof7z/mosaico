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
    /// Stable tmux pane id from $TMUX_PANE (e.g. "%5"). Present only when the
    /// hook fires inside a tmux session.
    #[serde(default)]
    tmux_pane: Option<String>,
    /// Value of $TMUX (socket path, session id, pane id). Used in meta JSON.
    #[serde(default)]
    tmux_socket: Option<String>,
    /// Harness-native resume token, supplied explicitly by programmatic hosts
    /// (opencode forwards its `ses_*` id here). For claude-code/codex this is
    /// absent — their adopted `session_id` IS the resume token (see below).
    #[serde(default)]
    resume_id: Option<String>,
    /// Which harness produced this hook (`claude-code`|`codex`|`opencode`). When
    /// absent, it is inferred from the id/resume shape for alias namespacing.
    #[serde(default)]
    harness: Option<String>,
    /// NIP-29 subgroup id (`h`) this pane was spawned into (from
    /// `TENEX_EDGE_CHANNEL`). When present, the session is scoped to this channel
    /// instead of the working-directory project: all channel publishing
    /// (presence/status/chat/mentions/membership) keys on it. The working
    /// directory remains the parent repo. Absent for ordinary project sessions.
    #[serde(default)]
    channel: Option<String>,
    /// Exact ordinal to allocate for this session (issue #47), forwarded from
    /// `TENEX_EDGE_ORDINAL` by a spawn-on-mention that targeted a specific
    /// `smithN`. When present, the signer honors it instead of lowest-free.
    #[serde(default)]
    preferred_ordinal: Option<u32>,
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
    // The NIP-29 channel this session belongs to. For a subgroup task room this is
    // the child `h` supplied via TENEX_EDGE_CHANNEL; otherwise it equals the
    // working-directory project (continuity: existing sessions are unchanged).
    // Everything below keys group membership + fabric publishing on `project`.
    let mut project = p
        .channel
        .clone()
        .filter(|g| !g.is_empty())
        .unwrap_or_else(|| work_root.clone());
    let rel_cwd = crate::project::rel_cwd(&cwd);
    let now = now_secs();
    if let Some(prog) = &progress {
        prog.emit(
            "project",
            format!("resolved project {project} from {}", cwd.display()),
        );
    }

    // Normalize the hook's identity inputs. claude-code/codex adopt their native
    // `session_id` (it doubles as the resume token); opencode supplies no
    // `session_id` and forwards its `ses_*` resume token instead. The harness
    // label is explicit when sent, else inferred from that shape (alias namespace
    // only — identity is the daemon-minted canonical id, never the harness id).
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
    let tmux_pane = p.tmux_pane.clone().filter(|s| !s.is_empty());

    // Per-session rooms (issue #6), gated by the `perSessionRooms` config
    // (default off): when ENABLED, a human-initiated session — one with no
    // TENEX_EDGE_CHANNEL override (someone ran `claude` / `tenex-edge launch`
    // directly) — lives in its OWN minted subgroup of the work-root project,
    // not the bare project. When DISABLED (default), such a session lands in
    // the bare project channel (`decide_session_room` returns UseExisting on
    // work_root, so `room_parent` stays None and nothing is minted).
    // Orchestration-spawned sessions (group override present) always join the
    // supplied subgroup, regardless of the flag. The room id is derived
    // deterministically from a stable per-session anchor so a resumed session
    // rejoins the SAME room; minting needs an operator key to sign the create.
    //
    // Anchor preference: the harness-native id (claude/codex) or resume token,
    // else the watched pid — opencode supplies neither id nor resume token at
    // start (only its pid), so the pid keeps it from being left in the bare
    // project. `room_parent` is `Some(parent_project)` exactly when we routed
    // the session into a freshly-minted room, and drives the create below.
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

    let obs = SessionObservation {
        agent_slug: p.agent.clone(),
        agent_pubkey: id.pubkey_hex(),
        project: project.clone(),
        host: state.host.clone(),
        rel_cwd: rel_cwd.clone(),
        harness,
        harness_session_id: harness_session_id.clone(),
        resume_id: resume_id.clone(),
        tmux_pane: tmux_pane.clone(),
        watch_pid: p.watch_pid,
        observed_at: now,
    };
    if let Some(prog) = &progress {
        prog.emit("session_registry", "registering or reasserting session");
    }
    // Canonical identity: the daemon MINTS a stable session id; the harness id /
    // resume token / pane / pid become rows in `session_aliases`. A reused
    // pane/pid slot occupied by a *different* logical session supersedes the old
    // one inside the registry (session_state lifecycle). NEVER adopt the raw
    // harness id as the identity.
    let snapshot = state.with_store(|s| s.register_or_reassert_session(&obs))?;
    let session_id = snapshot.session_id.as_str().to_owned();
    if let Some(prog) = &progress {
        prog.emit(
            "session_registry",
            format!(
                "session {} registered",
                crate::util::session_codename(&session_id)
            ),
        );
    }

    // The session's first kind:30315 row was enqueued by
    // register_or_reassert_session above. Do not wake the drainer until signer
    // selection below has reserved durable vs transient identity for this scope.

    // The resume token survives the session going dead so a later `tmux resume`
    // can reconstitute the harness: opencode's `ses_*`, else claude/codex native id.
    let resume_token: Option<String> = resume_id.clone().or_else(|| harness_session_id.clone());

    // A new logical session arriving on the SAME watched pid OR tmux pane (same
    // agent/project/host) means the harness restarted without a session-end. The
    // registry already superseded the stale `session_state` row; here we cancel
    // its engine task and mark its kept `sessions` runtime row dead so `who`
    // doesn't show ghosts.
    {
        let alive = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
        let mut stale_ids: Vec<String> = Vec::new();
        for rec in &alive {
            if rec.session_id == session_id || rec.agent_slug != p.agent || rec.host != state.host {
                continue;
            }
            let same_work_root = state
                .with_store(|s| {
                    let old_root = s.work_root_for_scope(&rec.project)?;
                    let new_root = room_parent.clone().unwrap_or_else(|| {
                        s.work_root_for_scope(&project).unwrap_or(project.clone())
                    });
                    Ok::<bool, anyhow::Error>(old_root == new_root)
                })
                .unwrap_or(rec.project == project);
            if !same_work_root {
                continue;
            }
            let same_pid = p.watch_pid.is_some() && rec.watch_pid == p.watch_pid;
            let same_pane = tmux_pane.as_deref().is_some_and(|pane| {
                state
                    .with_store(|s| s.get_session_endpoint(&rec.session_id, "tmux"))
                    .ok()
                    .flatten()
                    .map(|e| e.target)
                    .as_deref()
                    == Some(pane)
            });
            if same_pid || same_pane {
                // A restart on the same pid/pane is the SAME logical identity, not
                // a concurrent second personality — release the superseded
                // session's signer reservation NOW so the replacement reclaims the
                // durable signer slot (now shared, since same-agent sessions land
                // in the same project channel by default) instead of being forced
                // onto a transient key.
                state.release_session_signer(&rec.session_id);
                stale_ids.push(rec.session_id.clone());
            }
        }
        for old_id in stale_ids {
            cancel_session(state, &old_id);
            state.with_store(|s| {
                s.end_session(&old_id, now).ok();
                s.mark_session_dead(&old_id).ok();
            });
        }
    }

    // Atomic spawn reservation in the kept `sessions` runtime table, keyed by the
    // canonical id. This row carries the runtime-only detail (watch_pid, endpoints)
    // that `session_state` does not, and gates the idempotent re-start check below.
    state.with_store(|s| {
        s.upsert_session(&crate::state::SessionRecord {
            session_id: session_id.clone(),
            agent_slug: p.agent.clone(),
            agent_pubkey: id.pubkey_hex(),
            project: project.clone(),
            channel: p.channel.clone().unwrap_or_default(),
            host: state.host.clone(),
            child_pid: None,
            watch_pid: p.watch_pid,
            created_at: now,
            alive: true,
            rel_cwd: rel_cwd.clone(),
        })
        .ok();
        s.touch_session(&session_id, now).ok();
        // Persist the resume token (no-op when None/empty).
        if let Some(ref rt) = resume_token {
            s.set_session_resume_id(&session_id, rt).ok();
        }
        // Record the absolute path for this project so the tmux spawn command
        // can cd to it.
        s.upsert_project_path(&project, &cwd.to_string_lossy(), now)
            .ok();
        // Register the tmux endpoint if the hook env supplied TMUX_PANE.
        if let Some(ref pane) = tmux_pane {
            let meta = serde_json::json!({
                "socket": p.tmux_socket.as_deref().unwrap_or(""),
                "pane_command": p.agent,
            })
            .to_string();
            s.upsert_session_endpoint(&session_id, "tmux", pane, &meta, now)
                .ok();
        }
    });

    // Stamp the canonical session id onto the tmux session owning this pane so
    // the status-format `#(...)` can read it via `#{@te_session}` and pass
    // `--session` to `tenex-edge statusline`. Without this, two panes of the
    // same agent in the same project collapse to a single status bar (the
    // `#(...)` runs in the tmux server's env, which can't see the pane's
    // TENEX_EDGE_SESSION). Best-effort and deliberately outside the store lock.
    //
    // When the re-registration arrives without TMUX_PANE (e.g. a reassert from
    // a non-tmux context after a daemon restart), fall back to the session's
    // existing tmux endpoint so @te_session is never left stale.
    let effective_pane = tmux_pane.clone().or_else(|| {
        state
            .with_store(|s| s.get_session_endpoint(&session_id, "tmux"))
            .ok()
            .flatten()
            .map(|ep| ep.target)
    });
    if let Some(ref pane) = effective_pane {
        crate::tmux::set_pane_session_id(pane, &session_id, p.tmux_socket.as_deref());
    }

    // A session may acquire or refresh its tmux endpoint after unread rows were
    // already stored. Ring from the daemon on endpoint registration too, not
    // only from inbox write paths, so delivery does not depend on the tmux TUI
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

    // Make sure the project's NIP-29 group exists and this agent is a member
    // BEFORE the engine starts publishing, so its presence lands in a group it
    // already belongs to. Best-effort: never block a session from starting.
    if let Some(prog) = &progress {
        prog.emit(
            "nip29",
            "checking NIP-29 group state and membership on the relay",
        );
    }
    if let Some(parent) = &room_parent {
        // Human-initiated session: mint its per-session room under the work-root.
        // Mark the room in the LOCAL read-model SYNCHRONOUSLY (so the
        // room/subgroup gates and `groups list` recognize it immediately), then
        // do all the relay-dependent work (parent open, subgroup create, admin
        // reflection poll, member-add) in the BACKGROUND. This keeps session
        // start — and thus the first prompt — off the relay's critical path
        // entirely (fail-open). Chat into the room before the relay mint lands is
        // best-effort and simply not mirrored until the room exists.
        if let Some(prog) = &progress {
            prog.emit("nip29", format!("minting per-session room {project}"));
        }
        let now = now_secs();
        state.with_store(|s| {
            s.mark_session_room(&project, parent, now).ok();
            s.upsert_group_metadata(&project, &project, parent, now)
                .ok();
        });
        // ALL relay work (subgroup create, admin poll, member-add, subscription)
        // runs in the background — session start has zero synchronous relay await
        // on the room path, so a slow/unreachable relay never delays the engine
        // or the first prompt. ensure_session_room subscribes internally.
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
        // is a member. Bounded so a hung relay can't block session start (and the
        // hook that awaits it). On timeout the session still starts; membership
        // converges on the next start/heartbeat. Best-effort, fail-open.
        //
        // A top-level project is just the ROOT channel (parent_hint None); an
        // explicit channel scope (project != work_root) is a subgroup whose parent
        // project is ensured first (parent_hint = work_root). The SAME
        // `ensure_channel_ready` primitive handles both — there is no separate
        // "project" provisioning path.
        let parent_hint = if project != work_root && !work_root.is_empty() {
            Some(work_root.clone())
        } else {
            None
        };
        // Stamp the parent relationship immediately into the local DB so
        // `work_root_for_scope` returns the right project without waiting for
        // the relay to send back the kind:39000 with the `parent` tag.
        if let Some(ref parent) = parent_hint {
            state
                .with_store(|s| s.upsert_group_metadata(&project, &project, parent, now_secs()))
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
    let (harness_kind, native_id) =
        state.with_store(|s| s.get_session_derivation_anchor(&session_id));
    let signer = select_session_signer(
        state,
        &session_id,
        &id.keys,
        &id.pubkey_hex(),
        &p.agent,
        &project,
        &harness_kind,
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
            state.with_store(|s| {
                s.remove_session_pubkeys_for_session(&session_id).ok();
                s.mark_identity_route_dead(&session_id, now_secs()).ok();
            });
            return Err(e);
        }
    }

    // Nudge the drainer now that signer selection/admission is complete: the
    // pending first kind:30315 must be signed by the selected identity.
    state.status_outbox_notify.notify_waiters();

    // Keep the relay-authored group state (39000/39001/39002) subscribed so the
    // membership cache stays current — "check which groups we own at all times".
    if let Some(prog) = &progress {
        prog.emit(
            "subscription",
            "opening or refreshing project subscriptions",
        );
    }
    // Was the channel already subscribed before this session? If so, a mention
    // may have been published to it BEFORE this session existed (spawn-on-mention)
    // and the live materialize path — which only routes to sessions already alive
    // — never delivered it. We replay below once the session is alive. A channel
    // about to be freshly subscribed needs no replay: the relay streams its
    // backlog to this (already-alive) session as part of opening the new REQ.
    let needs_chat_replay = state
        .subscriptions
        .lock()
        .unwrap()
        .covers_channel(&project);
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
