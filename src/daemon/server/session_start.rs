use super::*;

mod abort;
mod advisory;
mod alias_resolution;
pub(crate) mod bootstrap;
mod channel_ready;
mod effects;
mod joined_channels;
mod lookup;
mod params;
mod reservation;
mod stale;

use abort::{abort_session_start, SessionStartGuard};
use alias_resolution::{record_secondary_aliases, resolve_session_id};
use lookup::{session_endpoint, work_root_for_scope};
use params::SessionStartParams;

pub(crate) use bootstrap::{bootstrap_exec_session_start, bootstrap_pty_session_start};
pub(in crate::daemon::server) use reservation::rpc_session_start;

pub(super) async fn rpc_session_start_inner(
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
    // Provision the agent keystore + spawn command and select its identity mode.
    let agent_identity = identity::load_or_create_with_command(
        &edge,
        &p.agent,
        now_secs(),
        p.provision_command.clone(),
    )?;
    let durable_agent = !agent_identity.per_session_key;
    validate_launch_reservation(state, &agent_identity, p.durable_reservation.as_deref())?;
    let cwd = p
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    // The working-directory channel (the repo this harness runs in).
    let work_root = crate::workspace::resolve(&cwd).unwrap_or_default();
    // The channel this session belongs to: the channel named by TENEX_EDGE_CHANNEL
    // for a task room, else the working-directory channel. The daemon is a direct
    // entry point (e2e drives `hook --type session-start` with a NAME in
    // TENEX_EDGE_CHANNEL), so resolve the NAME→opaque id through the ONE shared
    // resolver here. This is the single load-bearing conversion that kills the
    // name-vs-id double-create: every downstream consumer shares one id, and the
    // literal-name subgroup is never minted.
    let mut channel_provision_name: Option<String> = None;
    let mut channel = match p.channel.clone().filter(|g| !g.is_empty()) {
        // Session-start is a hook edge, so resolving a human channel name must not
        // await relay provisioning. Reserve one opaque id locally and let the
        // background readiness effect publish/materialize it.
        Some(name) => {
            let resolved = super::resolve_channel_for_session_start(state, &work_root, &name)
                .with_context(|| format!("resolving launch channel {name:?}"))?;
            channel_provision_name = resolved.provision_name;
            resolved.channel_h
        }
        None => work_root.clone(),
    };
    let rel_cwd = crate::workspace::rel_cwd(&cwd);
    let now = now_secs();
    if let Some(prog) = &progress {
        prog.emit(
            "channel",
            format!("resolved channel {channel} from {}", cwd.display()),
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
    tracing::debug!(agent = %p.agent, harness = %harness_str, channel = %channel, "session_start hook received");
    let pty_session = p.pty_session.clone().filter(|s| !s.is_empty());
    // The harness-native id to bind for resume: opencode `ses_*`, else claude/codex
    // native id.
    let native_id = resume_id
        .clone()
        .or_else(|| harness_session_id.clone())
        .unwrap_or_default();

    // Per-session rooms (issue #6), gated by `perSessionRooms` (default off). A
    // human-initiated session (no TENEX_EDGE_CHANNEL override) lives in its own
    // minted subgroup of the work-root when enabled, else the bare channel. The
    // room id is derived from a stable per-session anchor so a resumed session
    // rejoins the SAME room. `room_parent` is `Some(parent)` exactly when we routed
    // into a freshly-minted room.
    let pid_anchor = p.watch_pid.map(|pid| format!("pid-{pid}"));
    let room_parent: Option<String> = {
        let anchor = harness_session_id
            .as_deref()
            .or(resume_id.as_deref())
            .or(pty_session.as_deref())
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
                channel = crate::util::session_room_id(anchor);
                Some(parent)
            }
            _ => None,
        }
    };

    if let Some(prog) = &progress {
        prog.emit("session_registry", "registering or reasserting session");
    }
    // Resolve (or mint) the canonical session id WITHOUT writing the row yet. The
    // row is written further down, AFTER signer selection, so it is born carrying
    // this session's ordinal pubkey (never the base) — see the `upsert_session_row`
    // call below.
    let (session_id, ext_kind, ext_id) = resolve_session_id(
        state,
        harness_str,
        pty_session.as_deref(),
        harness_session_id.as_deref(),
        resume_id.as_deref(),
        p.watch_pid,
        durable_agent,
        now,
    )?;
    validate_agent_identity_admission(state, &session_id, &agent_identity)?;
    let already_running = state.sessions.lock().unwrap().contains_key(&session_id);
    let existing_session = already_running
        .then(|| state.with_store(|s| s.get_session(&session_id).ok().flatten()))
        .flatten();
    if let Some(existing) = existing_session.as_ref() {
        validate_live_session_identity(state, existing, &agent_identity)?;
    }
    if let Some(prog) = &progress {
        prog.emit(
            "session_registry",
            format!("session {session_id} registered"),
        );
    }

    record_secondary_aliases(
        state,
        harness_str,
        &session_id,
        pty_session.as_deref(),
        harness_session_id.as_deref(),
        resume_id.as_deref(),
        p.watch_pid,
        &work_root,
        &cwd,
        &channel,
        now,
    );

    membership_cleanup::cleanup_dead_local_sessions(state);

    // A new logical session arriving on the SAME watched pid OR PTY endpoint
    // (same agent, same work root) means the harness restarted without a session-end.
    // Cancel its engine task, release its signer reservation, and mark it dead so
    // `who` doesn't show ghosts. (All sessions in this DB are this machine's.)
    {
        let new_work_root = room_parent
            .clone()
            .unwrap_or_else(|| state.with_store(|s| work_root_for_scope(s, &channel)));
        stale::cancel_stale_sessions_on_restart(
            state,
            &session_id,
            &p.agent,
            p.watch_pid,
            pty_session.as_deref(),
            &new_work_root,
        );
    }

    let existing_channel = existing_session.map(|r| r.channel_h);
    if let Some(existing) = existing_channel.as_ref() {
        channel = existing.clone();
    }

    // Select the derived per-session or configured durable signer, then write
    // the row carrying that pubkey. Membership admission happens after channel-ready.
    let minted = mint_session_identity(
        state,
        &session_id,
        &agent_identity,
        &channel,
        SessionIdentityInput::new(&native_id, p.session_name.as_deref()),
        p.durable_reservation.as_deref(),
    )?;
    let mut start_guard = SessionStartGuard::new(state, &minted);
    retire_reclaimed_profile(state, minted.reclaimed_pubkey.as_deref()).await?;
    // If the engine is already running (re-assert from a duplicate spawn such as
    // the offline-agent-mention handler), preserve the live session's active
    // channel rather than stomping it with whatever TENEX_EDGE_CHANNEL the new
    // process was launched with. Without this guard, the duplicate's stale env
    // overwrites channel_h transiently AND permanently adds a spurious passive
    // join to session_channels (INSERT OR IGNORE never cleans it up), causing
    // the session to receive inbox messages from the wrong channel.
    let channel_for_upsert = existing_channel.unwrap_or_else(|| channel.clone());
    let effective_endpoint = pty_session
        .clone()
        .or_else(|| state.with_store(|s| session_endpoint(s, &session_id)));
    let needs_chat_replay = state.subs.lock().unwrap().covers_channel(&channel);
    let request = advisory::request_fact(
        &session_id,
        &p.agent,
        harness_str,
        ext_kind,
        ext_id.clone(),
        if durable_agent { "" } else { &native_id },
        &work_root,
        &channel,
        channel_for_upsert.clone(),
        &rel_cwd,
        room_parent.clone(),
        channel_provision_name.clone(),
        p.watch_pid,
        effective_endpoint.clone(),
        pty_session.is_some(),
        minted.identity.pubkey.clone(),
        minted.identity.display_slug(),
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
    let joined_channels = joined_channels::record(
        state,
        &session_id,
        channel_for_upsert.clone(),
        p.channels.clone(),
        now,
    );

    // Ring on endpoint registration so delivery does not depend on a later
    // mention event.
    if plan.ring_doorbell {
        crate::session_host::ring_doorbells(state.clone());
    }

    // Idempotent re-start (session reassert): the engine task already runs.
    if plan.reassert {
        tracing::info!(
            agent = %p.agent,
            session = %session_id,
            channel = %channel,
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
        start_guard.disarm();
        return Ok(serde_json::json!({
            "session_id": session_id,
        }));
    }

    // Session-start is the harness-critical edge: record local state and tell the
    // daemon what relay work needs doing, but never make the harness wait for
    // network proof or roster fan-out.
    if let Some(prog) = &progress {
        prog.emit("nip29", "scheduling channel readiness work");
    }
    effects::schedule_channel_ready(
        state.clone(),
        session_id.clone(),
        plan.channel_ready.clone(),
    );

    if plan.notify_outbox {
        state.outbox_notify.notify_waiters();
    }

    if let Some(prog) = &progress {
        prog.emit("subscription", "scheduling subscription work");
    }

    if plan.replay_chat {
        effects::schedule_replay_chat(state.clone(), channel.clone());
    }
    joined_channels::schedule_subscriptions(state, &joined_channels, &channel);

    let Some(spawn) = &plan.spawn else {
        anyhow::bail!("session_start advisory plan did not include spawn intent");
    };
    let ep = engine_params_for(
        &state.cfg,
        minted.identity.clone(),
        minted.keys.clone(),
        &spawn.session_id,
        &spawn.channel_h,
        &spawn.rel_cwd,
        p.dispatch_event.clone().filter(|s| !s.is_empty()),
        spawn.watch_pid,
    );
    if let Some(prog) = &progress {
        prog.emit("engine", "starting session engine and initial publishers");
    }
    if let Err(e) = spawn_session(state, ep).await {
        advisory::record_failed(state, &session_id, "spawn_engine", &e, now_secs());
        abort_session_start(state, &session_id);
        return Err(e);
    }
    tracing::info!(
        agent = %p.agent,
        channel = %channel,
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
            channel: channel.clone(),
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
    start_guard.disarm();

    Ok(serde_json::json!({
        "session_id": session_id,
    }))
}
