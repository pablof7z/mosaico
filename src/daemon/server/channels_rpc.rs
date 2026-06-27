use super::*;

pub(in crate::daemon::server) async fn ensure_session_room(
    state: &Arc<DaemonState>,
    room_h: &str,
    name: &str,
    parent: &str,
    agent_pubkey: &str,
) -> bool {
    // Record the session-room marker + hierarchy in the local read-model FIRST so
    // the `is_session_room`/`group_parent` gates (prompt+reply mirroring) and
    // `groups list` recognize the room even before — or if — the relay mint lands.
    state.with_store(|s| {
        s.mark_session_room(room_h, parent, now_secs()).ok();
        s.upsert_group_metadata(room_h, name, parent, now_secs())
            .ok();
    });

    // Provision the room through the SAME shared primitive every channel uses
    // (per-session rooms, orchestration task rooms, operator-created channels):
    // ensure the parent project exists (recursively), create+lock the subgroup,
    // propagate the parent's trusted admin set DOWN, and add the owning agent as a
    // member. Best-effort and fail-open — a degraded relay leaves the session
    // running without a relay-backed room.
    let gate = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: room_h,
            expect_member: agent_pubkey,
            parent_hint: Some(parent),
        })
        .await;
    let _ = ensure_subscription(state, room_h).await;

    // Name the room after the session's latest distilled title, if one exists.
    let latest_title =
        state.with_store(|s| s.latest_session_title_for_project(room_h).ok().flatten());
    if let Some(title) = latest_title {
        apply_room_name_update(state, room_h, &title).await;
    }

    !matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded)
}

pub(in crate::daemon::server) async fn rpc_channels_create(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use crate::fabric::nip29::orchestration::{build_add_agents_event, AddTarget};
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize)]
    struct AgentSpec {
        slug: String,
        backend: String,
    }
    #[derive(serde::Deserialize)]
    struct P {
        parent: String,
        name: String,
        #[serde(default)]
        agents: Vec<AgentSpec>,
        #[serde(default)]
        brief: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_create params")?;
    if p.agents.is_empty() {
        anyhow::bail!("at least one agent (slug@backend) is required");
    }

    // Relay subgroup-support verification is handled by a separate workstream;
    // call its gate here when it lands. For now we proceed and fail loudly below
    // if the relay rejects the subgroup create/lock.

    let nsec = state
        .cfg
        .management_nsec()
        .ok_or_else(|| anyhow::anyhow!("no signing key (tenexPrivateKey) set"))?;
    let mgmt_keys = Keys::parse(nsec).context("parsing signing key")?;
    let mgmt_pk = mgmt_keys.public_key().to_hex();

    // Short child id; hierarchy lives in metadata, not the id.
    let child_h = crate::util::child_group_id(&p.name);

    // Resolve each backend token to a hex pubkey. Accepts explicit
    // pubkey/npub/NIP-05 *and* host slugs as shown by `tenex-edge who`.
    let mut adds: Vec<AddTarget> = Vec::with_capacity(p.agents.len());
    for a in &p.agents {
        let backend_pubkey = resolve_backend_pubkey(state, &a.backend)
            .await
            .with_context(|| format!("resolving backend {:?}", a.backend))?;
        eprintln!(
            "[daemon] nip29-role-decision channel={} requested_agent={} backend={} backend_pubkey={} role=member reason=channels_create orchestration target; backend may be admin but spawned agent must be member",
            child_h,
            a.slug,
            a.backend,
            crate::util::pubkey_short(&backend_pubkey)
        );
        adds.push(AddTarget {
            backend_pubkey,
            slug: a.slug.clone(),
        });
    }

    // Resolve the creator agent (when invoked from a session) so the shared
    // provisioning primitive adds it as a member of the room it just made. A bare
    // operator invocation has none, in which case the management key (already the
    // group admin) is passed purely to provision the group.
    let creator: Option<String> = resolve_session(
        state,
        None,
        params.get("env_session").and_then(|v| v.as_str()),
        params.get("cwd").and_then(|v| v.as_str()),
        params.get("agent").and_then(|v| v.as_str()),
        None,
    )
    .ok()
    .map(|rec| rec.agent_pubkey);

    // Stamp the operator-chosen name + parent locally so the shared primitive
    // names the new subgroup correctly when it creates it on the relay (it reads
    // the display name from the local store).
    state.with_store(|s| {
        s.upsert_group_metadata(&child_h, &p.name, &p.parent, now_secs())
            .ok();
    });

    // ONE shared primitive provisions EVERY channel — per-session rooms,
    // orchestration task rooms, and operator-created channels are the same thing.
    // `ensure_channel_ready` ensures the parent project group exists (recursively),
    // creates+locks the child subgroup under it, propagates the trusted admin set
    // (parent admins + whitelist + backend) DOWN, and adds the member. The only
    // thing that differs between callers is where the name comes from and who the
    // member is. Fail loudly if the relay could not provision it.
    let expect_member = creator.as_deref().unwrap_or(&mgmt_pk);
    let gate = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: &child_h,
            expect_member,
            parent_hint: Some(&p.parent),
        })
        .await;
    if matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded) {
        anyhow::bail!(
            "relay did not provision subgroup {child_h} (parent {}); does the relay \
             support NIP-29 subgroups and is the signing key an admin?",
            p.parent
        );
    }
    let _ = ensure_subscription(state, &child_h).await;

    // The confirmed admin roster, read back from the local cache the shared
    // primitive just populated (parent admins + whitelist + backend pubkey).
    let granted: Vec<String> = state.with_store(|s| {
        s.list_group_members(&child_h)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, role)| role == "admin")
            .map(|(pk, _)| pk)
            .collect()
    });

    // Build + publish ONE kind:9 orchestration event into the parent (the
    // coordination group). The child id rides in an `h-target` tag.
    let prose = if p.brief.trim().is_empty() {
        generate_orchestration_prose(&adds)
    } else {
        p.brief.clone()
    };
    let builder = build_add_agents_event(&p.parent, &child_h, &adds, &prose)?;
    let signed = state.transport.sign(builder, &mgmt_keys).await?;
    let orchestration_event_id = signed.id.to_hex();
    // Checked publish: the bare `publish_event` resolves `Ok` even when every
    // relay rejected the kind:9 (NIP-29 `blocked` / rate-limited), so reporting
    // `orchestration_event_id` off it would advertise a channel whose
    // orchestration event was silently dropped — backends would never receive
    // the add-agents directive. `publish_event_checked` turns a relay rejection
    // into a hard error so `channels_create` fails loudly instead of lying
    // about success.
    state.transport.publish_event_checked(&signed).await?;

    // Local fast-path: relays don't reliably echo to the publishing connection,
    // so drive the same listener directly for roles targeted at THIS backend.
    // Idempotency is enforced inside handle_orchestration via processed_orchestration.
    if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(&signed) {
        handle_orchestration(state, &signed, op).await;
    }

    Ok(serde_json::json!({
        "child_h": child_h,
        "display_path": format!("{} > {}", p.parent, p.name),
        "admins": granted,
        "creator": creator.unwrap_or_default(),
        "orchestration_event_id": orchestration_event_id,
    }))
}

/// `channels list`: render the subgroup tree under `project` from LOCAL daemon
/// state (materialized kind:39000 metadata) — no relay round-trip. Returns the
/// rooms in depth-first order, each with a `depth` (the project root is depth 0
/// and not included; its direct children are depth 1) so the CLI can indent.
pub(in crate::daemon::server) fn rpc_channels_list(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_list params")?;

    // (group_id, about, name, parent) for every group the daemon knows about.
    let rows = state.with_store(|s| s.list_group_metadata())?;
    // parent id -> children (id, display name). Sorted for stable output.
    let mut children: std::collections::BTreeMap<String, Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for (id, about, name, parent) in &rows {
        if parent.is_empty() {
            continue;
        }
        let display = if name.is_empty() {
            about.clone()
        } else {
            name.clone()
        };
        children
            .entry(parent.clone())
            .or_default()
            .push((id.clone(), display));
    }
    for v in children.values_mut() {
        v.sort();
    }

    let rooms = preorder_rooms(&children, &p.project);
    Ok(serde_json::json!({ "project": p.project, "rooms": rooms }))
}

/// Pre-order DFS flatten of the subgroup tree rooted at `root` into
/// `{child_h, name, depth}` JSON (root excluded, its children at depth 0).
pub(in crate::daemon::server) fn preorder_rooms(
    children: &std::collections::BTreeMap<String, Vec<(String, String)>>,
    root: &str,
) -> Vec<serde_json::Value> {
    fn walk(
        children: &std::collections::BTreeMap<String, Vec<(String, String)>>,
        node: &str,
        depth: usize,
        seen: &mut std::collections::HashSet<String>,
        out: &mut Vec<serde_json::Value>,
    ) {
        if let Some(kids) = children.get(node) {
            for (child_id, name) in kids {
                if !seen.insert(child_id.clone()) {
                    continue;
                }
                out.push(serde_json::json!({
                    "child_h": child_id,
                    "name": name,
                    "depth": depth,
                }));
                walk(children, child_id, depth + 1, seen, out);
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(root.to_string());
    walk(children, root, 0, &mut seen, &mut out);
    out
}

/// `channels_switch`: move a running session to a different NIP-29 subgroup
/// without restarting. Writes `sessions.channel`, re-points `session_state.
/// project` at the new scope (so the status drainer, `who`/`statusline`
/// scoping, and `status_delta_since` all key on it), bumps the state version,
/// and enqueues a status_outbox row so a fresh kind:30315 publishes into the
/// new room. All fabric publishing (chat/mentions/proposals), local chat
/// routing, and turn-context deltas follow the new scope via `route_scope()`.
/// Fails if the channel does not exist in local state.
pub(in crate::daemon::server) async fn rpc_channels_switch(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        #[serde(default)]
        env_session: Option<String>,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_switch params")?;
    if p.channel.trim().is_empty() {
        anyhow::bail!("channel h must not be empty");
    }
    let env_session = p
        .env_session
        .as_deref()
        .filter(|s| !s.is_empty())
        .context("channels switch must be run from within a tenex-edge agent session (TENEX_EDGE_SESSION is not set)")?;
    let rec = resolve_session(state, None, Some(env_session), None, None, None)?;
    let new_channel = p.channel.clone();
    // Validate the channel exists in local state before switching.
    let exists: bool =
        state.with_store(|s| Ok::<bool, anyhow::Error>(s.channel_exists(&new_channel)))?;
    if !exists {
        anyhow::bail!("channel {:?} does not exist", new_channel);
    }
    refresh_project_members_cache(state, &new_channel).await;
    let is_member = state.with_store(|s| {
        Ok::<bool, anyhow::Error>(
            s.is_group_member(&new_channel, &rec.agent_pubkey)
                .unwrap_or(false),
        )
    })?;
    if !is_member {
        anyhow::bail!(
            "agent {} is not a member of channel {:?}",
            rec.agent_slug,
            new_channel
        );
    }
    // Occupancy reject (issue #47): a session's ordinal identity is fixed for
    // life; it cannot switch INTO a room where another live session already holds
    // the same ordinal pubkey. Redirect the agent to message that instance — the
    // redirect lands in that instance's context because both sign as this pubkey.
    let my_pubkey = state
        .with_store(|s| s.identity_route_for_session(&rec.session_id))
        .map(|r| r.pubkey)
        .unwrap_or_else(|| rec.agent_pubkey.clone());
    if let Some(occupant) = state.with_store(|s| s.live_identity_route(&my_pubkey, &new_channel)) {
        if occupant.session_id != rec.session_id {
            anyhow::bail!(
                "Another instance of you is already active in #{new_channel}, so you cannot join it. \
Send it a message instead: tenex-edge chat write --channel {new_channel} --message \"...\" \
— it will arrive in the context of the instance working there."
            );
        }
    }
    ensure_subscription(state, &new_channel).await?;
    let prev_channel = rec.channel.clone();
    // Apply the switch in ONE store transaction: update `sessions.channel`,
    // move `session_state.project` to the new scope (so the status drainer, who
    // snapshot, and status_delta all key on it), bump the version, and enqueue
    // a status_outbox row so a fresh kind:30315 publishes into the new room.
    // Without this, `channels switch` only flipped a column and the session
    // kept routing into its old per-session room.
    state.with_store(|s| s.set_session_channel(&rec.session_id, &new_channel, now_secs()))?;
    // Move the (pubkey, h) identity route to the new room — the ordinal pubkey is
    // fixed, only `h` changes; `(pubkey, new_channel)` becomes the resume key.
    state.with_store(|s| s.move_identity_route(&rec.session_id, &new_channel, now_secs()).ok());
    // Nudge the drainer so the scope-changed status publishes immediately
    // rather than waiting for the next heartbeat tick. The kind:30315 it
    // publishes carries the new `h` tag, so peers in the channel see the
    // session's presence without a separate profile push.
    state.status_outbox_notify.notify_waiters();
    Ok(serde_json::json!({
        "session_id": rec.session_id,
        "prev_channel": prev_channel,
        "channel": new_channel,
    }))
}

/// Human-readable summary of the add-agents request, grouped per backend, e.g.
/// "@<edge1>: add research-lead. @<edge2>: add implementation-lead and test1."
/// Advisory only — receivers act on the structured tags, never this prose.
pub(in crate::daemon::server) fn generate_orchestration_prose(
    adds: &[crate::fabric::nip29::orchestration::AddTarget],
) -> String {
    use std::collections::BTreeMap;
    let mut by_backend: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for a in adds {
        by_backend
            .entry(a.backend_pubkey.as_str())
            .or_default()
            .push(a.slug.as_str());
    }
    let mut parts: Vec<String> = Vec::new();
    for (backend, slugs) in by_backend {
        parts.push(format!(
            "@{}: add {}.",
            crate::util::pubkey_short(backend),
            slugs.join(" and ")
        ));
    }
    parts.join(" ")
}
