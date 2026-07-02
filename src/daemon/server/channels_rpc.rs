use super::channel_membership_rpc::set_active_session_channel;
use super::*;

pub(in crate::daemon::server) async fn ensure_session_room(
    state: &Arc<DaemonState>,
    room_h: &str,
    name: &str,
    parent: &str,
    agent_pubkey: &str,
) -> bool {
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
            // The intended room name rides on the create publish; the relay's
            // kind:39000 echo is what lands it in the cache.
            name: Some(name),
            repair_whitelisted_admins: true,
        })
        .await;
    let _ = ensure_subscription(state, room_h).await;

    // The channel `name` is set ONLY at create (or explicit edit) — never from a
    // session's distilled title — so there is no room auto-rename here.

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
        /// Explicit literal parent group h. Set by the launch picker, operator
        /// invocations, and tests. When absent, the parent defaults to the
        /// creating agent's CURRENT channel (see `parent` resolution below).
        #[serde(default)]
        parent: Option<String>,
        /// Project-relative parent override from `channels create
        /// --parent-channel`. Resolved within the creator's project subtree; takes
        /// precedence over both the literal `parent` and the current-channel
        /// default.
        #[serde(default)]
        parent_channel: Option<String>,
        name: String,
        #[serde(default)]
        agents: Vec<AgentSpec>,
        /// Durable channel description, published to the relay as kind:39000
        /// `about`. Set at creation; never derived from the name.
        #[serde(default)]
        about: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_create params")?;

    // Resolve the creator agent (when invoked from a session) FIRST — both the
    // child-of-current-channel default and the auto-switch below need it. Strict
    // resolution (no project fallback): child-of-current and auto-switch must only
    // fire when actually run as an agent, never bind to an arbitrary sibling
    // session of a bare operator invocation.
    let creator_rec = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    )
    .ok();

    // Operator cwd-resolved project slug (== root channel_h for project roots).
    // Used as fallback when there is no agent session.
    let cwd_project: Option<String> = params["cwd"]
        .as_str()
        .filter(|s| !s.is_empty())
        .and_then(|cwd| crate::project::resolve(std::path::Path::new(cwd)).ok());

    // Resolve the parent the new channel hangs under:
    //   1. `--parent-channel <ref>` — project-relative override.
    //   2. the creating agent's CURRENT channel — the child-of-current default.
    //   3. an explicit literal `parent` — the picker / operator / test path.
    //   4. cwd-resolved project root — bare operator invocation from a project dir.
    let parent: String = if let Some(r) = p
        .parent_channel
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Prefer the session's project root; fall back to cwd-resolved project.
        let root = if let Some(rec) = &creator_rec {
            state.with_store(|s| super::project_root(s, &rec.channel_h))
        } else {
            cwd_project.clone().context(
                "--parent-channel requires running inside an agent session or a project directory",
            )?
        };
        match state.with_store(|s| super::resolve_channel_ref(s, &root, r)) {
            super::ChannelResolution::Unique(h) => h,
            super::ChannelResolution::Ambiguous(refs) => {
                return Ok(serde_json::json!({ "ambiguous": refs, "reference": r }));
            }
            super::ChannelResolution::NotFound => {
                anyhow::bail!("no channel matching {r:?} in this project")
            }
        }
    } else if let Some(rec) = &creator_rec {
        rec.channel_h.clone()
    } else if let Some(par) = p.parent.as_deref().filter(|s| !s.is_empty()) {
        par.to_string()
    } else if let Some(proj) = cwd_project {
        proj
    } else {
        anyhow::bail!(
            "channels create needs a parent: run it inside an agent session, pass --parent-channel, or run from a project directory"
        );
    };

    // Names are unique per parent: a `create --name X` where X already exists is
    // an ERROR, not a silent no-op — the agent needs to KNOW the channel is already
    // there (so it switches in rather than assuming it minted a fresh one). Point
    // it at the existing channel with a copy-paste switch command.
    if let Some(existing) = state.with_store(|s| s.channel_id_for_name(&parent, &p.name))? {
        anyhow::bail!(
            "channel {:?} already exists under this parent (id {existing}). \
Switch into it instead: tenex-edge channels switch {}",
            p.name,
            p.name
        );
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

    // Opaque random child id; the human handle lives in the kind:39000 `name`,
    // never in the id, and the hierarchy lives in the `parent` metadata.
    let child_h = crate::util::opaque_group_id();

    // Resolve each backend label to the backend's pubkey. The label is the raw
    // config.json `backendName`, not a pubkey, NIP-05, or OS/DNS hostname.
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
            session_id: None,
        });
    }

    // The creator's pubkey (resolved above) tells the shared provisioning
    // primitive to add it as a member of the room it just made. A bare operator
    // invocation has none, in which case the management key (already the group
    // admin) is passed purely to provision the group.
    let creator: Option<String> = creator_rec.as_ref().map(|rec| rec.agent_pubkey.clone());

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
            parent_hint: Some(&parent),
            // Operator-chosen name rides on the create publish; the relay's
            // kind:39000 echo lands it in the cache (no local fabrication).
            name: Some(&p.name),
            repair_whitelisted_admins: true,
        })
        .await;
    if matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded) {
        anyhow::bail!(
            "relay did not provision subgroup {child_h} (parent {parent}); does the relay \
             support NIP-29 subgroups and is the signing key an admin?"
        );
    }
    let _ = ensure_subscription(state, &child_h).await;

    // Publish the durable `about` as kind:9002 edit-metadata so it reaches the
    // relay's kind:39000 (not just the local cache), signed by the management key
    // exactly like `rpc_project_edit` does. Best-effort: the channel exists either
    // way; an unset `about` skips the publish.
    if !p.about.trim().is_empty() {
        let builder = crate::fabric::nip29::lifecycle::group_edit_metadata(&child_h, &p.about)?;
        let _ = state.transport.publish_signed(builder, &mgmt_keys).await;
        // Re-read the relay's now-updated kind:39000 so the `about` lands in the
        // cache from relay truth, not a local write.
        let _ = state.provider.fetch_and_materialize_channel(&child_h).await;
    }

    // The confirmed admin roster, read back from the local cache the shared
    // primitive just populated (parent admins + whitelist + backend pubkey).
    let granted: Vec<String> = state.with_store(|s| {
        s.list_channel_members(&child_h)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.role == "admin")
            .map(|m| m.pubkey)
            .collect()
    });

    // Build + publish ONE kind:9 orchestration event into the parent (the
    // coordination group), but ONLY when agents were named — `--agent` is
    // optional, and an add-agents event with no `add` tags is meaningless (no
    // backend would act on it). An empty channel is created and joined without
    // any orchestration. The child id rides in an `h-target` tag.
    let orchestration_event_id = if adds.is_empty() {
        String::new()
    } else {
        let prose = generate_orchestration_prose(&adds);
        let builder = build_add_agents_event(&parent, &child_h, &adds, &prose)?;
        let signed = state.transport.sign(builder, &mgmt_keys).await?;
        let oid = signed.id.to_hex();
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
        // Idempotency is enforced per add-target inside handle_orchestration.
        if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(&signed) {
            handle_orchestration(state, &signed, op).await;
        }
        oid
    };

    // Auto-focus: join the new room and make it the active publishing channel.
    // Unlike `channels switch`, this preserves the parent as a passive joined
    // channel so the creator can still see and receive mentions from it.
    let switched = if let Some(rec) = &creator_rec {
        set_active_session_channel(state, &rec.session_id, &rec.agent_pubkey, &child_h, false)?;
        true
    } else {
        false
    };

    Ok(serde_json::json!({
        "child_h": child_h,
        "display_path": format!("{} > {}", parent, p.name),
        "admins": granted,
        "creator": creator.unwrap_or_default(),
        "switched": switched,
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

    // Every channel the daemon has materialized.
    let rows = state.with_store(|s| s.list_channels())?;
    // parent id -> children (id, display name, about). Sorted for stable output.
    let mut children: std::collections::BTreeMap<String, Vec<(String, String, String)>> =
        std::collections::BTreeMap::new();
    for ch in &rows {
        if ch.parent.is_empty() {
            continue;
        }
        let display = if ch.name.is_empty() {
            ch.about.clone()
        } else {
            ch.name.clone()
        };
        children.entry(ch.parent.clone()).or_default().push((
            ch.channel_h.clone(),
            display,
            ch.about.clone(),
        ));
    }
    for v in children.values_mut() {
        v.sort();
    }

    let rooms = preorder_rooms(&children, &p.project);
    Ok(serde_json::json!({ "project": p.project, "rooms": rooms }))
}

/// Pre-order DFS flatten of the subgroup tree rooted at `root` into
/// `{child_h, name, about, depth}` JSON (root excluded, its children at depth 0).
pub(in crate::daemon::server) fn preorder_rooms(
    children: &std::collections::BTreeMap<String, Vec<(String, String, String)>>,
    root: &str,
) -> Vec<serde_json::Value> {
    fn walk(
        children: &std::collections::BTreeMap<String, Vec<(String, String, String)>>,
        node: &str,
        depth: usize,
        seen: &mut std::collections::HashSet<String>,
        out: &mut Vec<serde_json::Value>,
    ) {
        if let Some(kids) = children.get(node) {
            for (child_id, name, about) in kids {
                if !seen.insert(child_id.clone()) {
                    continue;
                }
                out.push(serde_json::json!({
                    "child_h": child_id,
                    "name": name,
                    "about": about,
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
