use super::*;
use crate::fabric::nip29::orchestration::{build_admit_running_event, AddTarget};

#[derive(serde::Deserialize)]
struct AcceptParams {
    name: String,
    about: String,
}

pub(in crate::daemon::server) async fn rpc_accept(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: AcceptParams = serde_json::from_value(params.clone()).context("channel_move_accept")?;
    let name = p.name.trim();
    if name.is_empty() || name.contains('.') || name.contains('/') {
        anyhow::bail!("--yes-lets-move requires one concise child-channel name");
    }
    let about = p.about.trim();
    if about.is_empty() {
        anyhow::bail!("--yes-lets-move requires a non-empty channel about");
    }
    crate::channel_about::validate_channel_about(about)
        .context("--yes-lets-move about is invalid")?;
    let caller = resolve_session(state, &CallerAnchor::from_params(params))?;
    let now = now_secs();
    let Some(offer) = current_offer(state, &caller.pubkey, now) else {
        anyhow::bail!(
            "no current channel-move suggestion for this session; wait for a new topology nudge"
        );
    };
    let existing =
        state.with_store(|store| store.channel_id_for_name(&offer.evidence.parent, name))?;
    let resuming_partial_move = caller.channel_h != offer.evidence.parent
        && existing.as_deref() == Some(caller.channel_h.as_str());
    if !caller_can_resume_offer(
        &caller.channel_h,
        &offer.evidence.parent,
        existing.as_deref(),
    ) {
        clear_offer(state, &caller.pubkey);
        anyhow::bail!("channel-move suggestion is stale because this session changed channels");
    }
    if !resuming_partial_move {
        let Some(current) = current_evidence(state, &offer.evidence.parent, now)? else {
            clear_offer(state, &caller.pubkey);
            anyhow::bail!("channel-move suggestion is stale because the conversation changed");
        };
        if !same_cohort(&offer.evidence, &current) {
            clear_offer(state, &caller.pubkey);
            anyhow::bail!(
                "channel-move suggestion is stale because the participant cohort changed"
            );
        }
    }

    let (child_h, created) = match existing {
        Some(child_h) => {
            channel_membership_rpc::ensure_joinable(state, &caller, &child_h).await?;
            channel_membership_rpc::set_active_session_channel(state, &caller.pubkey, &child_h)?;
            (child_h, false)
        }
        None => {
            let create_params = move_create_params(params, &offer.evidence.parent, name, about);
            let created = channels_rpc::rpc_channel_create(state, &create_params).await?;
            let child_h = created["child_h"]
                .as_str()
                .context("channel create returned no child id")?
                .to_string();
            (child_h, true)
        }
    };

    let mut added = vec![caller.pubkey.clone()];
    let mut requested = Vec::<String>::new();
    let mut skipped = Vec::<serde_json::Value>::new();
    let mut remote_targets = Vec::<AddTarget>::new();
    for participant in &offer.evidence.cohort {
        if participant.pubkey == caller.pubkey {
            continue;
        }
        if participant.host == state.host {
            match current_local_participant(state, participant, &offer.evidence.parent)? {
                Some(session) => {
                    if let Err(error) =
                        channel_membership_rpc::ensure_joinable(state, &session, &child_h).await
                    {
                        skipped.push(skip(participant, format!("{error:#}")));
                    } else {
                        added.push(participant.pubkey.clone());
                    }
                }
                None => skipped.push(skip(participant, "session is no longer running")),
            }
            continue;
        }
        let backend_pubkey = match resolve_backend_pubkey(state, &participant.host).await {
            Ok(pubkey) => pubkey,
            Err(error) => {
                skipped.push(skip(participant, format!("{error:#}")));
                continue;
            }
        };
        if let Err(error) = invite_rpc::ensure_backend_admin(state, &child_h, &backend_pubkey).await
        {
            skipped.push(skip(participant, format!("{error:#}")));
            continue;
        }
        remote_targets.push(AddTarget {
            backend_pubkey,
            slug: participant.label.clone(),
            session_pubkey: Some(participant.pubkey.clone()),
        });
    }
    if !remote_targets.is_empty() {
        match publish_running_only_moves(
            state,
            &offer.evidence.parent,
            &child_h,
            name,
            &remote_targets,
        )
        .await
        {
            Ok(_) => requested.extend(
                remote_targets
                    .iter()
                    .filter_map(|target| target.session_pubkey.clone()),
            ),
            Err(error) => {
                for target in &remote_targets {
                    let participant = offer
                        .evidence
                        .cohort
                        .iter()
                        .find(|participant| {
                            target.session_pubkey.as_deref() == Some(&participant.pubkey)
                        })
                        .expect("remote target came from cohort");
                    skipped.push(skip(participant, format!("{error:#}")));
                }
            }
        }
    }
    subscriptions::reconcile_subs_logged(state, "channel_move_accept").await;

    let pointer = format!("Moving this to #{name}");
    let pointer_posted =
        if pointer_exists(state, &offer.evidence.parent, &pointer, offer.offered_at)? {
            false
        } else {
            let mut send_params = params.clone();
            if let Some(obj) = send_params.as_object_mut() {
                obj.insert("message".into(), serde_json::json!(pointer));
                obj.insert("channel".into(), serde_json::json!(offer.evidence.parent));
                obj.insert("tags".into(), serde_json::json!([]));
                obj.insert("force".into(), serde_json::json!(false));
                obj.insert("attachments".into(), serde_json::json!([]));
            }
            channel_send::rpc_channel_send(state, &send_params).await?;
            true
        };
    clear_offer(state, &caller.pubkey);
    Ok(serde_json::json!({
        "parent": offer.evidence.parent,
        "child_h": child_h,
        "name": name,
        "created": created,
        "added": added,
        "requested": requested,
        "skipped": skipped,
        "pointer_posted": pointer_posted,
        "child_seed_posted": false,
    }))
}

fn move_create_params(
    params: &serde_json::Value,
    parent: &str,
    name: &str,
    about: &str,
) -> serde_json::Value {
    let mut create_params = params.clone();
    let obj = create_params
        .as_object_mut()
        .expect("validated channel move params are an object");
    obj.insert("parent".into(), serde_json::json!(parent));
    obj.insert("name".into(), serde_json::json!(name));
    obj.insert("about".into(), serde_json::json!(about));
    obj.insert("agents".into(), serde_json::json!([]));
    create_params
}

fn caller_can_resume_offer(current: &str, parent: &str, existing_child: Option<&str>) -> bool {
    current == parent || existing_child == Some(current)
}

fn same_cohort(captured: &ConversationEvidence, current: &ConversationEvidence) -> bool {
    captured.parent == current.parent
        && captured.cohort.len() == current.cohort.len()
        && captured.cohort.iter().all(|participant| {
            current.cohort.iter().any(|candidate| {
                candidate.pubkey == participant.pubkey
                    && candidate.runtime_generation == participant.runtime_generation
                    && candidate.host == participant.host
            })
        })
}

fn current_local_participant(
    state: &Arc<DaemonState>,
    participant: &ParticipantSnapshot,
    parent: &str,
) -> Result<Option<crate::state::Session>> {
    state.with_store(|store| {
        let Some(session) = store.get_session(&participant.pubkey)? else {
            return Ok(None);
        };
        let generation_matches = participant.runtime_generation == Some(session.runtime_generation);
        let still_in_parent = session.channel_h == parent
            || store
                .has_session_route(&session.pubkey, parent)
                .unwrap_or(false);
        Ok((session.is_running() && generation_matches && still_in_parent).then_some(session))
    })
}

async fn publish_running_only_moves(
    state: &Arc<DaemonState>,
    parent: &str,
    child_h: &str,
    name: &str,
    targets: &[AddTarget],
) -> Result<String> {
    let keys = state.management_keys()?;
    let prose = format!("Move the active conversation into #{name}");
    let builder = build_admit_running_event(parent, child_h, targets, &prose)?;
    let signed = state.nmp.sign_event(builder, &keys).await?;
    let event_id = signed.id.to_hex();
    state.nmp.publish_group_event(&signed, true).await?;
    if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(&signed) {
        handle_orchestration(state, &signed, op).await;
    }
    Ok(event_id)
}

fn pointer_exists(
    state: &Arc<DaemonState>,
    parent: &str,
    pointer: &str,
    since: u64,
) -> Result<bool> {
    Ok(state
        .with_store(|store| {
            store.recent_chat_messages_for_channel(parent, since.saturating_sub(1), 200)
        })?
        .iter()
        .any(|message| message.sync_state == "accepted" && message.body.trim() == pointer))
}

fn skip(participant: &ParticipantSnapshot, reason: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "pubkey": participant.pubkey,
        "label": participant.label,
        "reason": reason.into(),
    })
}

#[cfg(test)]
#[path = "accept/tests.rs"]
mod tests;
