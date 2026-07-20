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

pub(crate) use bootstrap::bootstrap_hosted_session_start;
pub(in crate::daemon::server) use reservation::rpc_session_start;

pub(super) async fn rpc_session_start_inner(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    progress: Option<InitProgress>,
) -> Result<serde_json::Value> {
    progress_emit(&progress, "session_start", "parsing hook payload");
    let mut p: SessionStartParams =
        serde_json::from_value(params.clone()).context("parsing session_start params")?;
    if p.agent.trim().is_empty() {
        anyhow::bail!("session_start requires an agent slug");
    }

    let mosaico_home = config::mosaico_home();
    config::ensure_dir(&mosaico_home)?;
    let facts = params::runtime_facts(&p)?;
    let harness = facts.observed_harness;
    let cwd = p
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    state.refresh_agent_catalog()?;
    let work_root = crate::daemon::workspace_path::channel_for_path(&cwd)?;
    let rel_cwd = crate::workspace::rel_cwd(&cwd)?;
    runtime::bind_workspace(state, &cwd, &work_root)?;
    let harness_name = harness.as_str();
    let located_pubkey = runtime::resolve_existing_pubkey(state, &p, harness_name)?;
    let persisted = runtime::reconcile_agent_from_pubkey(state, &mut p, located_pubkey.as_deref())?;
    let agent = if let Some(existing) = persisted {
        if state.with_store(|store| store.is_derived_session_pubkey(&existing.pubkey))? {
            identity::AgentIdentity::per_session(&existing.agent_slug, harness_name)
        } else {
            identity::load(&mosaico_home, &existing.agent_slug).with_context(|| {
                format!("loading persisted agent identity {:?}", existing.agent_slug)
            })?
        }
    } else if identity::is_configured(&mosaico_home, &p.agent) {
        identity::load(&mosaico_home, &p.agent).with_context(|| {
            located_pubkey.as_ref().map_or_else(
                || format!("loading agent identity {:?}", p.agent),
                |_| {
                    format!(
                        "identity configuration changed for live agent {:?}",
                        p.agent
                    )
                },
            )
        })?
    } else if state
        .resolve_native_agent(&p.agent, Some(&cwd), Some(harness))
        .is_ok()
        || (state.installed_harnesses().contains(&harness) && p.agent == harness.agent_slug())
    {
        identity::AgentIdentity::per_session(&p.agent, harness.as_str())
    } else {
        state.mutate_agent_config(|| {
            identity::load_or_create(
                &mosaico_home,
                &p.agent,
                harness.as_str(),
                p.profile.as_deref(),
                now_secs(),
            )
        })?
    };
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
    )
    .await?;
    let existing = state.with_store(|store| store.get_session(&pubkey))?;
    let already_running = existing
        .as_ref()
        .is_some_and(|session| session.is_running())
        && state.runtime.engines.lock().unwrap().contains_key(&pubkey);
    if already_running {
        if let Some(existing) = &existing {
            channel = existing.channel_h.clone();
        }
    }
    let readiness_parent = channel_ready::session_parent_hint(
        state,
        &channel,
        &work_root,
        room_parent.as_deref(),
        existing.as_ref(),
    )?;
    let now = now_secs();
    let runtime_generation =
        runtime::reserve_generation(state, &p, &facts, &pubkey, &channel, now, existing.as_ref())?;
    let mut reservation = (!already_running)
        .then(|| RuntimeReservation::new(state.clone(), pubkey.clone(), runtime_generation));

    state.with_store(|store| -> Result<()> {
        store.set_session_context(&pubkey, &channel, &work_root, &readiness_parent)?;
        if !store.bind_runtime_process(&pubkey, runtime_generation, p.watch_pid)? {
            anyhow::bail!(
                "runtime generation {runtime_generation} for {pubkey} is no longer active"
            );
        }
        store.record_claimed_harness(&pubkey, &facts.claimed_harness)?;
        let admitted = store
            .get_session(&pubkey)?
            .context("reserved session disappeared before locator binding")?;
        if !facts.claimed_harness.is_empty() && facts.claimed_harness != admitted.observed_harness {
            tracing::warn!(
                pubkey = %pubkey,
                claimed_harness = %facts.claimed_harness,
                observed_harness = %admitted.observed_harness,
                "hook harness claim differs from admitted observation"
            );
        }
        runtime::bind_locators(store, &p, &admitted.observed_harness, &pubkey, now)?;
        Ok(())
    })?;
    retire_reclaimed_profile(state, reclaimed_pubkey.as_deref()).await?;

    let endpoint = p.pty_session.clone().filter(|value| !value.is_empty());
    let request = advisory::SessionStartRequest {
        pubkey: pubkey.clone(),
        channel_h: channel.clone(),
        rel_cwd: rel_cwd.clone(),
        room_parent,
        readiness_parent: (!readiness_parent.is_empty()).then_some(readiness_parent.clone()),
        channel_provision_name,
        watch_pid: p.watch_pid,
        pty_session: endpoint,
        ring_doorbell: p.pty_session.is_some(),
        already_running,
        channel_already_subscribed: state
            .subscriptions
            .reconciler
            .lock()
            .unwrap()
            .covers_channel(&channel),
    };
    let plan = advisory::plan(&request);
    let joined = joined_channels::record(state, &pubkey, channel.clone(), p.channels, now);

    if plan.ring_doorbell {
        crate::session_host::ring_doorbells(state.clone());
    }
    if plan.reassert {
        if let Some(guard) = reservation.as_mut() {
            guard.disarm();
        }
        return Ok(serde_json::json!({ "pubkey": pubkey }));
    }

    let lifecycle_epoch = state
        .with_store(|store| store.get_session(&pubkey))?
        .map(|session| session.lifecycle_epoch)
        .context("reserved session disappeared before channel readiness")?;
    effects::schedule_channel_ready(
        state.clone(),
        pubkey.clone(),
        runtime_generation,
        lifecycle_epoch,
        plan.channel_ready,
    );
    if plan.replay_chat {
        effects::schedule_replay_chat(state.clone(), channel.clone());
    }
    joined_channels::schedule_admission(
        state.clone(),
        pubkey.clone(),
        runtime_generation,
        lifecycle_epoch,
        &joined,
        &channel,
    );

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
    spawn_session(state, engine).await?;
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
    if let Some(guard) = reservation.as_mut() {
        guard.disarm();
    }
    Ok(serde_json::json!({ "pubkey": pubkey }))
}

fn resolve_start_channel(
    _state: &Arc<DaemonState>,
    p: &SessionStartParams,
    work_root: &str,
) -> Result<(String, Option<String>)> {
    let Some(channel_h) = p.channel.as_deref().filter(|value| !value.is_empty()) else {
        return Ok((work_root.to_string(), None));
    };
    Ok((channel_h.to_string(), None))
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
            let _ = self.state.with_store(|store| {
                store.mark_runtime_stopped_if_generation(
                    &self.pubkey,
                    self.generation,
                    crate::state::StopReason::Unknown,
                    now_secs(),
                )
            });
        }
    }
}
