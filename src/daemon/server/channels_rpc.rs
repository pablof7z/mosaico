use super::channel_membership_rpc::{resolve_caller, resolve_target_channel};
use super::*;
use crate::fabric::nip29::lifecycle::as_nostr;

const CHANNEL_CREATE_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

pub(in crate::daemon::server) async fn ensure_session_room(
    state: &Arc<DaemonState>,
    room_h: &str,
    name: &str,
    parent: &str,
    member_pubkey: &str,
) -> crate::fabric::nip29::readiness::ChannelGate {
    // Provision the room through the SAME shared primitive every channel uses
    // (per-session rooms, orchestration task rooms, operator-created channels):
    // ensure the parent channel exists (recursively), create+lock the subgroup,
    // propagate the parent's trusted admin set DOWN, and add the owning agent as a
    // member. Best-effort and fail-open — a degraded relay leaves the session
    // running without a relay-backed room.
    let gate = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: room_h,
            expect_member: member_pubkey,
            parent_hint: Some(parent),
            // The intended room name rides on the create publish; the relay's
            // kind:39000 echo is what lands it in the cache.
            name: Some(name),
            repair_whitelisted_admins: true,
        })
        .await;
    let _ = ensure_subscription(state, room_h).await;

    // The channel `name` is set ONLY at create (or explicit edit) — never from a
    // session's agent-supplied title — so there is no room auto-rename here.

    gate
}

pub(in crate::daemon::server) async fn rpc_channel_create(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use crate::fabric::nip29::orchestration::{build_add_agents_event, AddTarget};
    #[derive(serde::Deserialize)]
    struct AgentSpec {
        slug: String,
        backend: String,
    }
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        #[serde(default)]
        agents: Vec<AgentSpec>,
        /// Durable channel description, published to the relay as kind:39000
        /// `about`. Set at creation; never derived from the name.
        #[serde(default)]
        about: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_create params")?;
    crate::channel_about::validate_channel_about(&p.about)?;
    let (parent_ref, name) = crate::channel_ref::split_create_path(&p.channel)?;

    // Resolve the creator only to join it to the new channel. Creation never
    // changes a session's channel memberships except for this additive join.
    let creator_rec = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    )
    .ok();

    let parent = match state.with_store(|s| absolute::resolve_absolute_channel_ref(s, &parent_ref))
    {
        super::ChannelResolution::Unique(h) => h,
        super::ChannelResolution::NotFound => {
            anyhow::bail!(
                "{}",
                state.with_store(|s| absolute::describe_missing_channel(s, &parent_ref))
            )
        }
    };

    let workspace_root =
        state.with_store(|store| match store.get_channel(&parent).ok().flatten() {
            Some(channel) if channel.parent.is_empty() => Some(channel.channel_h),
            None => Some(parent.clone()),
            _ => None,
        });
    crate::channel_name::validate_child(&name, workspace_root.as_deref())?;

    // Names are unique per parent. A duplicate is an error, not a silent no-op.
    if let Some(existing) = state.with_store(|s| s.channel_id_for_name(&parent, &name))? {
        let existing = state
            .with_store(|store| super::channel_resolve::channel_reference_for(store, &existing))?;
        anyhow::bail!("channel {existing} already exists");
    }

    // Relay subgroup-support verification is handled by a separate workstream;
    // call its gate here when it lands. For now we proceed and fail loudly below
    // if the relay rejects the subgroup create/lock.

    let mgmt_keys = state.management_keys()?;
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
            "[daemon] nip29-role-decision channel={} requested_agent={} backend={} backend_pubkey={} role=member reason=channel_create orchestration target; backend may be admin but spawned agent must be member",
            child_h,
            a.slug,
            a.backend,
            crate::util::pubkey_short(&backend_pubkey)
        );
        adds.push(AddTarget {
            backend_pubkey,
            slug: a.slug.clone(),
            session_pubkey: None,
        });
    }

    // The creator's pubkey (resolved above) tells the shared provisioning
    // primitive to add it as a member of the room it just made. A bare operator
    // invocation has none, in which case the management key (already the group
    // admin) is passed purely to provision the group.
    let creator: Option<String> = creator_rec.as_ref().map(|rec| rec.pubkey.clone());

    // ONE shared primitive provisions EVERY channel — per-session rooms,
    // orchestration task rooms, and operator-created channels are the same thing.
    // `ensure_channel_ready` ensures the parent channel group exists (recursively),
    // creates+locks the child subgroup under it, propagates the trusted admin set
    // (parent admins + whitelist + backend) DOWN, and adds the member. The only
    // thing that differs between callers is where the name comes from and who the
    // member is. Fail loudly if the relay could not provision it.
    let expect_member = creator.as_deref().unwrap_or(&mgmt_pk);
    let standing_lane = state.standing_sync.lock().await;
    let ready = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: &child_h,
            expect_member,
            parent_hint: Some(&parent),
            // Operator-chosen name rides on the create publish; the relay's
            // kind:39000 echo lands it in the cache (no local fabrication).
            name: Some(&name),
            repair_whitelisted_admins: true,
        });
    let gate = tokio::time::timeout(CHANNEL_CREATE_READY_TIMEOUT, ready)
        .await
        .with_context(|| {
            format!(
                "channel_create timed out provisioning {name:?} after {}s",
                CHANNEL_CREATE_READY_TIMEOUT.as_secs()
            )
        })?;
    let parent_ref =
        state.with_store(|store| super::channel_resolve::channel_reference_for(store, &parent))?;
    gate.require_ready(format!(
        "channel_create could not provision {name:?} below {parent_ref}"
    ))?;
    if let Some(rec) = creator_rec.as_ref() {
        let recorded = super::managed_lifecycle::commit_confirmed_admission(
            state,
            &rec.pubkey,
            &child_h,
            rec.runtime_generation,
            rec.lifecycle_epoch,
        )
        .await?;
        if !recorded {
            anyhow::bail!("creator session changed while channel membership was being confirmed");
        }
    }
    drop(standing_lane);
    let _ = ensure_subscription(state, &child_h).await;

    // Publish the durable `about` as kind:9002 edit-metadata so it reaches the
    // relay's kind:39000 (not just the local cache), signed by the management key
    // exactly like the channel edit RPC does. Best-effort: the channel exists either
    // way; an unset `about` skips the publish.
    if !p.about.trim().is_empty() {
        let builder = as_nostr(nmp_nip29::edit_metadata(nmp_nip29::GroupMetadataEdit {
            about: Some(p.about.clone()),
            ..nmp_nip29::GroupMetadataEdit::default()
        }));
        let _ = state.nmp.publish_group(&child_h, builder, &mgmt_keys);
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
        // Durable acceptance is the reporting boundary: NMP has taken custody
        // of the add-agents directive and will keep delivering it. Whether each
        // relay took it is settlement, and settlement is inspected -- through
        // the publish queue `mosaico doctor` reads -- never awaited here.
        //
        // The directive reaches THIS backend's own orchestration listener the
        // same way a peer's does: NMP injects the accepted row into the group
        // subscription and `demux::chat_ops` routes it (NMP #1182). There is no
        // local fast-path, because there is no longer anything for it to fix.
        state
            .nmp
            .publish_group(&parent, builder, &mgmt_keys)?
            .to_hex()
    };

    let joined = creator_rec.is_some();

    let channel =
        state.with_store(|store| super::channel_resolve::channel_reference_for(store, &child_h))?;
    Ok(serde_json::json!({
        "channel": channel,
        "admins": granted,
        "creator": creator.unwrap_or_default(),
        "joined": joined,
        "orchestration_event_id": orchestration_event_id,
    }))
}

mod archive;
pub(in crate::daemon::server) use archive::{archive_channel, rpc_channel_archive};

mod delete;
pub(in crate::daemon::server) use delete::rpc_channel_delete;

mod list;
pub(in crate::daemon::server) use list::rpc_channel_list;

mod edit;
pub(in crate::daemon::server) use edit::rpc_channel_edit;

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
