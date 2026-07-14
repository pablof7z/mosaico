use super::*;

mod advisory;
pub(crate) mod bootstrap;
mod channel_ready;
mod effects;
mod joined_channels;
mod params;
mod replacement;
mod reservation;
mod runtime;

use params::SessionStartParams;

pub(crate) use bootstrap::{bootstrap_exec_session_start, bootstrap_pty_session_start};
pub(in crate::daemon::server) use reservation::rpc_session_start;

pub(super) async fn rpc_session_start_inner(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    progress: Option<InitProgress>,
) -> Result<serde_json::Value> {
    progress_emit(&progress, "session_start", "parsing hook payload");
    let p: SessionStartParams =
        serde_json::from_value(params.clone()).context("parsing session_start params")?;
    if p.agent.trim().is_empty() {
        anyhow::bail!("session_start requires an agent slug");
    }

    let edge = config::edge_home();
    config::ensure_dir(&edge)?;
    let agent = identity::load_or_create_with_command(
        &edge,
        &p.agent,
        now_secs(),
        p.provision_command.clone(),
    )?;
    let cwd = p
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let work_root = crate::workspace::resolve(&cwd).unwrap_or_default();
    let rel_cwd = crate::workspace::rel_cwd(&cwd);
    runtime::bind_workspace(state, &cwd, &work_root)?;
    let harness = resolve_harness(&p);
    let harness_name = harness.as_str();
    let native_resume = p
        .resume_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            p.harness_session
                .as_deref()
                .filter(|value| !value.is_empty())
        });

    let located_pubkey = runtime::resolve_existing_pubkey(state, &p, harness_name)?;
    let prepared = match located_pubkey.as_deref() {
        Some(pubkey) => load_session_identity(state, pubkey, &agent)?,
        None => prepare_session_identity(state, &agent, p.session_name.as_deref())?,
    };
    let pubkey = prepared.identity.pubkey.clone();
    let reclaimed_pubkey = p
        .reclaimed_pubkey
        .as_deref()
        .or(prepared.reclaimed_pubkey.as_deref())
        .map(str::to_string);

    let (mut channel, channel_provision_name) = resolve_start_channel(state, &p, &work_root)?;
    let room_parent = if p.channel.as_deref().is_none_or(str::is_empty)
        && state.per_session_rooms()
        && !work_root.is_empty()
    {
        channel = crate::util::session_room_id(&pubkey);
        Some(work_root.clone())
    } else {
        None
    };

    replacement::retire_conflicting_pid_runtime(
        state,
        &pubkey,
        &p.agent,
        harness_name,
        p.watch_pid,
        &work_root,
    )?;
    let existing = state.with_store(|store| store.get_session(&pubkey))?;
    let already_running = existing.as_ref().is_some_and(|session| session.alive)
        && state.sessions.lock().unwrap().contains_key(&pubkey);
    if already_running {
        if let Some(existing) = &existing {
            channel = existing.channel_h.clone();
        }
    }
    let now = now_secs();
    let runtime_generation = runtime::reserve_generation(
        state,
        &p,
        harness_name,
        &pubkey,
        &channel,
        now,
        existing.as_ref(),
    )?;
    let mut reservation = (!already_running)
        .then(|| RuntimeReservation::new(state.clone(), pubkey.clone(), runtime_generation));

    state.with_store(|store| -> Result<()> {
        store.set_session_channel(&pubkey, &channel)?;
        if !store.bind_runtime_process(&pubkey, runtime_generation, p.watch_pid)? {
            anyhow::bail!(
                "runtime generation {runtime_generation} for {pubkey} is no longer active"
            );
        }
        bind_locators(store, &p, harness_name, &pubkey, now)?;
        Ok(())
    })?;
    retire_reclaimed_profile(state, reclaimed_pubkey.as_deref()).await?;
    membership_cleanup::cleanup_dead_local_sessions(state);

    let endpoint = p.pty_session.clone().filter(|value| !value.is_empty());
    let request = crate::reconcile::SessionStartRequestFact {
        pubkey: pubkey.clone(),
        agent: p.agent.clone(),
        harness: harness_name.to_string(),
        native_id: native_resume.unwrap_or_default().to_string(),
        work_root: work_root.clone(),
        channel_h: channel.clone(),
        channel_for_upsert: channel.clone(),
        rel_cwd: rel_cwd.clone(),
        room_parent,
        channel_provision_name,
        watch_pid: p.watch_pid,
        pty_session: endpoint,
        ring_doorbell: p.pty_session.is_some(),
        signer_label: prepared.identity.display_slug(),
        already_running,
        channel_already_subscribed: state.subs.lock().unwrap().covers_channel(&channel),
        at: now,
    };
    let plan = advisory::drive_request(state, request)?.plan;
    let joined = joined_channels::record(state, &pubkey, channel.clone(), p.channels, now);

    if plan.ring_doorbell {
        crate::session_host::ring_doorbells(state.clone());
    }
    if plan.reassert {
        advisory::record_started(state, &pubkey, &channel, p.watch_pid, now_secs());
        if let Some(guard) = reservation.as_mut() {
            guard.disarm();
        }
        return Ok(serde_json::json!({ "pubkey": pubkey }));
    }

    effects::schedule_channel_ready(state.clone(), pubkey.clone(), plan.channel_ready);
    if plan.notify_outbox {
        state.outbox_notify.notify_waiters();
    }
    if plan.replay_chat {
        effects::schedule_replay_chat(state.clone(), channel.clone());
    }
    joined_channels::schedule_subscriptions(state, &joined, &channel);

    let spawn = plan
        .spawn
        .context("session_start advisory plan did not include spawn intent")?;
    let engine = engine_params_for(
        &state.cfg,
        prepared.identity,
        prepared.keys,
        runtime_generation,
        &spawn.channel_h,
        &work_root,
        &spawn.rel_cwd,
        p.dispatch_event.filter(|value| !value.is_empty()),
        spawn.watch_pid,
    );
    progress_emit(&progress, "engine", "starting session engine");
    if let Err(error) = spawn_session(state, engine).await {
        advisory::record_failed(state, &pubkey, "spawn_engine", &error, now_secs());
        return Err(error);
    }
    if plan.emit_tail {
        state.emit_tail(TailEvent::Sess {
            ts: now_secs(),
            channel: channel.clone(),
            agent: p.agent,
            session: pubkey.clone(),
            state: "start".into(),
            rel_cwd,
        });
    }
    advisory::record_started(state, &pubkey, &channel, p.watch_pid, now_secs());
    if let Some(guard) = reservation.as_mut() {
        guard.disarm();
    }
    Ok(serde_json::json!({ "pubkey": pubkey }))
}

fn resolve_harness(p: &SessionStartParams) -> Harness {
    p.harness
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(Harness::from_str)
        .unwrap_or_else(|| {
            if p.resume_id.is_some() {
                Harness::Opencode
            } else if p.harness_session.is_some() {
                Harness::ClaudeCode
            } else {
                Harness::Unknown
            }
        })
}

fn resolve_start_channel(
    state: &Arc<DaemonState>,
    p: &SessionStartParams,
    work_root: &str,
) -> Result<(String, Option<String>)> {
    let Some(name) = p.channel.as_deref().filter(|value| !value.is_empty()) else {
        return Ok((work_root.to_string(), None));
    };
    let resolved = resolve_channel_for_session_start(state, work_root, name)
        .with_context(|| format!("resolving launch channel {name:?}"))?;
    Ok((resolved.channel_h, resolved.provision_name))
}

fn bind_locators(
    store: &crate::state::Store,
    p: &SessionStartParams,
    harness: &str,
    pubkey: &str,
    now: u64,
) -> Result<()> {
    if let Some(endpoint) = p.pty_session.as_deref().filter(|value| !value.is_empty()) {
        let kind = if p.endpoint_kind.as_deref() == Some("acp") {
            crate::state::LOCATOR_ACP
        } else {
            crate::state::LOCATOR_PTY
        };
        store.put_session_locator(harness, kind, endpoint, pubkey, now)?;
    }
    if let Some(native) = p
        .resume_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            p.harness_session
                .as_deref()
                .filter(|value| !value.is_empty())
        })
    {
        store.set_native_resume_locator(pubkey, harness, native, now)?;
    }
    if let Some(pid) = p.watch_pid {
        store.put_session_locator(
            harness,
            crate::state::LOCATOR_PID,
            &pid.to_string(),
            pubkey,
            now,
        )?;
    }
    Ok(())
}

fn progress_emit(progress: &Option<InitProgress>, stage: &str, message: &str) {
    if let Some(progress) = progress {
        progress.emit(stage, message);
    }
}

struct RuntimeReservation {
    state: Arc<DaemonState>,
    pubkey: String,
    generation: u64,
    armed: bool,
}

impl RuntimeReservation {
    fn new(state: Arc<DaemonState>, pubkey: String, generation: u64) -> Self {
        Self {
            state,
            pubkey,
            generation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RuntimeReservation {
    fn drop(&mut self) {
        if self.armed {
            let _ = self
                .state
                .with_store(|store| store.mark_dead_if_generation(&self.pubkey, self.generation));
        }
    }
}
