use super::*;

// ── relay demux: one subscription, route to all hosted agents ────────────────

pub(super) fn spawn_demux(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut notifications = state.transport.notifications();
        loop {
            let ev: Option<Event> = match notifications.recv().await {
                Ok(RelayPoolNotification::Event { event, .. }) => Some(*event),
                Ok(RelayPoolNotification::Message {
                    message: RelayMessage::Event { event, .. },
                    ..
                }) => Some(event.into_owned()),
                Ok(_) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(_) => None,
            };
            if let Some(event) = ev {
                handle_incoming(&state, &event);
            }
        }
    });
}

/// Decode one event and apply it. Multi-agent aware: "me" is the SET of hosted
/// local pubkeys; a mention routes by `to_pubkey` to that agent's sessions only.
fn handle_incoming(state: &Arc<DaemonState>, event: &Event) {
    // Deduplicate: multiple overlapping REQ subscriptions (one per agent×project)
    // cause the relay to deliver the same event N times. Process each event ID once.
    {
        let id = event.id.to_hex();
        let mut seen = state.seen_events.lock().unwrap();
        if seen.contains(&id) {
            return;
        }
        if seen.len() >= 512 {
            seen.pop_front();
        }
        seen.push_back(id);
    }

    // NIP-29 group metadata cache (kind:39000, relay-authored).
    if event.kind.as_u16() == 39000 {
        if let (Some(project), about) = (
            event_tag(event, "d"),
            event_tag(event, "about").unwrap_or(""),
        ) {
            state.with_store(|s| {
                s.upsert_project_meta(project, about, event.created_at.as_secs())
                    .ok();
            });
        }
        return;
    }
    // NIP-29 membership snapshot (kind:39002, relay-authored). Replace the cached
    // member set for the group so it self-heals against the relay's authority.
    if event.kind.as_u16() == 39002 {
        if let Some(project) = event_tag(event, "d") {
            let members: Vec<(String, String)> = event
                .tags
                .iter()
                .filter_map(|t| {
                    let s = t.as_slice();
                    if s.first().map(String::as_str) == Some("p") {
                        s.get(1).map(|pk| {
                            (
                                pk.clone(),
                                s.get(2).cloned().unwrap_or_else(|| "member".to_string()),
                            )
                        })
                    } else {
                        None
                    }
                })
                .collect();
            state.with_store(|s| {
                s.replace_group_members(project, &members, event.created_at.as_secs())
                    .ok();
            });
        }
        return;
    }
    let Some(de) = state.codec.decode(event) else {
        return;
    };
    let _ = state.tail_tx_send(de.clone());

    let hosted = state.hosted_pubkeys();
    let is_self = hosted.contains(&event.pubkey.to_hex());
    let now = now_secs();

    match de {
        DomainEvent::Profile(_)
        | DomainEvent::Presence(_)
        | DomainEvent::Activity(_)
        | DomainEvent::Status(_)
        | DomainEvent::TurnReply(_)
            if is_self => {}
        DomainEvent::Profile(pf) => {
            let pk = pf.agent.pubkey.clone();
            if crate::acl::is_allowed(&pk) {
                state.with_store(|s| {
                    s.upsert_profile(&pk, &pf.agent.slug, &pf.host, now).ok();
                    s.remove_pending_agent(&pk).ok();
                });
            } else if !crate::acl::is_blocked(&pk)
                && pf.owners.iter().any(|o| state.owners.contains(o))
            {
                state.with_store(|s| {
                    s.upsert_pending_agent(
                        &pk,
                        &pf.agent.slug,
                        &pf.host,
                        &pf.owners.join(","),
                        now,
                    )
                    .ok();
                });
            }
        }
        DomainEvent::Presence(pr) => {
            if pr.expires_at <= now {
                return;
            }
            state.with_store(|s| {
                s.upsert_peer_session(
                    pr.session_id.as_str(),
                    &pr.agent.pubkey,
                    &pr.agent.slug,
                    &pr.project,
                    &pr.host,
                    &pr.rel_cwd,
                    now,
                )
                .ok();
                if !pr.agent.slug.is_empty() {
                    s.upsert_profile(&pr.agent.pubkey, &pr.agent.slug, &pr.host, now)
                        .ok();
                }
            });
        }
        DomainEvent::Status(st) => {
            if st.expires_at.map(|e| e <= now).unwrap_or(false) {
                return;
            }
            state.with_store(|s| {
                s.set_agent_status(&st.agent.pubkey, &st.project, &st.text, now)
                    .ok();
            });
        }
        DomainEvent::Mention(m) if hosted.contains(&m.to_pubkey) => {
            // User-prompt events (operator-signed) must not enter the inbox.
            if !state.owners.contains(&event.pubkey.to_hex()) {
                let to = m.to_pubkey.clone();
                let routed = state.with_store(|s| route_mention_into(s, &to, &m, event));
                if routed {
                    state.mention_notify.notify_waiters();
                }
            }
        }
        _ => {}
    }
}

pub(super) fn event_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

// ── pruner ───────────────────────────────────────────────────────────────────

pub(super) fn spawn_pruner(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let before = now_secs().saturating_sub(PRUNE_PEER_AFTER_SECS);
            state.with_store(|s| {
                let _ = s.prune_peer_sessions(before);
            });
        }
    });
}

// ── idle-exit watcher ─────────────────────────────────────────────────────────

pub(super) fn spawn_idle_watcher(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        loop {
            state.liveness_changed.notified().await;
            if is_idle(&state) {
                tokio::select! {
                    _ = tokio::time::sleep(grace()) => {
                        if is_idle(&state) {
                            eprintln!("[daemon] idle for grace period; exiting");
                            state.shutdown.notify_waiters();
                            return;
                        }
                    }
                    _ = state.liveness_changed.notified() => {}
                }
            }
        }
    });
}

fn is_idle(state: &Arc<DaemonState>) -> bool {
    *state.open_clients.lock().unwrap() == 0 && state.live_session_count() == 0
}
