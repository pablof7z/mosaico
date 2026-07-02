use super::Nip29Provider;
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate};
use crate::util::now_secs;
use std::future::Future;
use std::pin::Pin;

impl Nip29Provider {
    /// Ensure `ctx.channel` exists on the relay and has `ctx.expect_member`.
    pub async fn ensure_channel_ready<'a>(&'a self, ctx: ChannelCtx<'a>) -> ChannelGate {
        // Never provision an empty channel id: a 9007 create-group with an empty
        // `h`/`d` mints a junk relay group (kind:39000 with d="") and a bogus
        // empty-channel_h cache row. An empty scope means "no channel resolved",
        // which is a caller bug, not a group to create — fail closed.
        if ctx.channel.trim().is_empty() {
            eprintln!("[daemon] ensure_channel_ready: refusing to provision an empty channel id");
            return ChannelGate::Degraded;
        }
        ensure_channel_ready_inner(self, ctx, 0).await
    }
}

fn ensure_channel_ready_inner<'a>(
    provider: &'a Nip29Provider,
    ctx: ChannelCtx<'a>,
    depth: u8,
) -> Pin<Box<dyn Future<Output = ChannelGate> + Send + 'a>> {
    Box::pin(async move {
        if depth > 3 {
            eprintln!(
                "[daemon] ensure_channel_ready: recursion depth limit reached for {:?}",
                ctx.channel
            );
            return ChannelGate::Degraded;
        }

        // Normalize: Some("") is the DB's sentinel for "known root channel" but
        // is meaningless as a provisioning parent. Treat it as None (no parent)
        // so callers that read channel_parent() without filtering cannot feed an
        // empty h into group creation, even on the recursive path.
        let parent_hint = ctx.parent_hint.filter(|h| !h.is_empty());

        let (is_ready, inflight) = provider.readiness.check(ctx.channel, ctx.expect_member);
        if is_ready {
            return ChannelGate::Ready;
        }

        let _guard = inflight.lock().await;
        let (is_ready, _) = provider.readiness.check(ctx.channel, ctx.expect_member);
        if is_ready {
            return ChannelGate::Ready;
        }

        let Some(mgmt_keys) = provider.parse_management_keys() else {
            return ChannelGate::Degraded;
        };
        let mgmt_pubkey = mgmt_keys.public_key().to_hex();

        let parent_admins: Vec<String> = if let Some(parent) = parent_hint {
            let grandparent = provider
                .with_store(|s| s.channel_parent(parent).unwrap_or(None))
                .filter(|p| !p.is_empty());
            let parent_ctx = ChannelCtx {
                channel: parent,
                expect_member: &mgmt_pubkey,
                parent_hint: grandparent.as_deref(),
                name: None,
                repair_whitelisted_admins: ctx.repair_whitelisted_admins,
            };
            let parent_gate = ensure_channel_ready_inner(provider, parent_ctx, depth + 1).await;
            if matches!(parent_gate, ChannelGate::Degraded) {
                eprintln!(
                    "[daemon] ensure_channel_ready: parent {:?} is degraded; aborting for {:?}",
                    parent, ctx.channel
                );
                return ChannelGate::Degraded;
            }
            provider.with_store(|s| {
                s.list_channel_members(parent)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| m.role == "admin")
                    .map(|m| m.pubkey)
                    .collect()
            })
        } else {
            vec![]
        };

        // A relay fetch FAILURE must never be read as "group absent" — that would
        // drive spurious group re-creation (fabrication-by-omission). Degrade
        // loudly without attempting to create anything.
        let (group_exists, roles, members) = match provider.try_fetch_group_state(ctx.channel).await
        {
            Ok(state) => state,
            Err(e) => {
                tracing::error!(
                    channel = ctx.channel,
                    error = %format!("{e:#}"),
                    "ensure_channel_ready: relay fetch failed — degrading without attempting creation (no fabrication-by-omission)"
                );
                return ChannelGate::Degraded;
            }
        };
        let mut repaired = false;

        if !group_exists {
            let created = if let Some(parent) = parent_hint {
                // The subgroup's display NAME rides on the create publish (9002
                // metadata) so the relay's authored kind:39000 carries it. It is
                // NEVER stashed in `relay_channels` first — that cache is fed only
                // by materializing relay events. An unnamed session room (no name
                // from the caller) names itself after its own id.
                let name = ctx.name.filter(|n| !n.is_empty()).unwrap_or(ctx.channel);
                provider
                    .nip29_create_subgroup(ctx.channel, name, parent)
                    .await
            } else {
                // A root group names itself after its slug (group_lock_closed emits
                // `["name", slug]`); the relay's kind:39000 echoes it back.
                let ok = match crate::fabric::nip29::lifecycle::group_create(ctx.channel) {
                    Ok(b) => {
                        provider
                            .publish_group_management(b, &mgmt_keys, "9007 create-group")
                            .await
                    }
                    Err(e) => {
                        tracing::error!(
                            channel = ctx.channel,
                            error = %format!("{e:#}"),
                            "ensure_channel_ready: group_create build failed — cannot provision root group"
                        );
                        false
                    }
                };
                if ok {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if let Ok(b) = crate::fabric::nip29::lifecycle::group_lock_closed(ctx.channel) {
                        provider
                            .publish_group_management(b, &mgmt_keys, "9002 lock-closed")
                            .await;
                    }
                }
                ok
            };

            if created {
                repaired = true;
                // Enter the channel into the cache by reading back the relay's OWN
                // kind:39000 (await the echo) — never a local optimistic write. If
                // it never materializes, fail loud and degrade.
                let mut materialized = false;
                for attempt in 0..12u32 {
                    if provider.fetch_and_materialize_channel(ctx.channel).await {
                        materialized = true;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(
                        250 * (attempt as u64 + 1).min(3),
                    ))
                    .await;
                }
                if !materialized {
                    eprintln!(
                        "[daemon] ensure_channel_ready: kind:39000 for {:?} did not materialize \
                         after create; degrading (no local fabrication)",
                        ctx.channel
                    );
                    return ChannelGate::Degraded;
                }
                for attempt in 0..6u32 {
                    let roles_now = provider.fetch_group_roles(ctx.channel).await;
                    if roles_now.get(&mgmt_pubkey).map(String::as_str) == Some("admin") {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(
                        250 * (attempt as u64 + 1).min(3),
                    ))
                    .await;
                }
            } else if !provider.fetch_and_materialize_channel(ctx.channel).await {
                // Creation was rejected AND the group is absent from the relay —
                // nothing to provision against; give up.
                eprintln!(
                    "[daemon] ensure_channel_ready: failed to create {:?}",
                    ctx.channel
                );
                return ChannelGate::Degraded;
            }
            // else: group pre-existed on the relay (create rejected because it was
            // already there); fall through to membership / admin checks.
        } else if roles.get(&mgmt_pubkey).map(String::as_str) != Some("admin") {
            let granted = provider
                .try_grant_mgmt_admin_via_user_nsec(ctx.channel, &mgmt_pubkey)
                .await;
            if !granted {
                eprintln!(
                    "[daemon] ensure_channel_ready: management key is not admin of {:?} \
                     and self-grant failed",
                    ctx.channel
                );
                return ChannelGate::Degraded;
            }
        }

        // SOOT guarantee: a ready channel must be present in `relay_channels` from
        // the relay's OWN kind:39000 — not a local optimistic write. A freshly
        // created group was already materialized above; a pre-existing group hit by
        // a cold daemon cache is read back from the relay here (best-effort).
        if provider.with_store(|s| s.get_channel(ctx.channel).ok().flatten().is_none()) {
            provider.fetch_and_materialize_channel(ctx.channel).await;
        }

        let mut invariant_ok = true;
        {
            let mut required_admins: Vec<String> = vec![mgmt_pubkey.clone()];
            if ctx.repair_whitelisted_admins {
                required_admins.extend(provider.whitelisted_pubkeys.iter().cloned());
            }
            for pk in &parent_admins {
                if !required_admins.contains(pk) {
                    required_admins.push(pk.clone());
                }
            }
            for pk in &required_admins {
                let already_admin = roles.get(pk.as_str()).map(String::as_str) == Some("admin");
                if already_admin {
                    continue;
                }
                if confirm_role_grant(provider, ctx.channel, pk, true).await {
                    provider.with_store(|s| {
                        if let Err(e) = s.upsert_channel_member(ctx.channel, pk, "admin", now_secs())
                        {
                            tracing::error!(
                                channel = ctx.channel,
                                pubkey = pk.as_str(),
                                error = %e,
                                "ensure_channel_ready: local admin mirror write failed after confirmed relay grant — cache divergence"
                            );
                        }
                    });
                    repaired = true;
                } else {
                    eprintln!(
                        "[daemon] ensure_channel_ready: admin grant for {pk} in {:?} not confirmed on the relay",
                        ctx.channel
                    );
                    invariant_ok = false;
                }
            }
        }

        if !invariant_ok {
            return ChannelGate::Degraded;
        }

        let expect_already_admin = mgmt_pubkey == ctx.expect_member
            || provider
                .whitelisted_pubkeys
                .iter()
                .any(|pk| pk == ctx.expect_member)
            || parent_admins.iter().any(|pk| pk == ctx.expect_member);
        if !expect_already_admin
            && !members.contains(ctx.expect_member)
            && !roles.contains_key(ctx.expect_member)
        {
            if confirm_role_grant(provider, ctx.channel, ctx.expect_member, false).await {
                provider.with_store(|s| {
                    if let Err(e) =
                        s.upsert_channel_member(ctx.channel, ctx.expect_member, "member", now_secs())
                    {
                        tracing::error!(
                            channel = ctx.channel,
                            pubkey = ctx.expect_member,
                            error = %e,
                            "ensure_channel_ready: local member mirror write failed after confirmed relay grant — cache divergence"
                        );
                    }
                });
                repaired = true;
            } else {
                eprintln!(
                    "[daemon] ensure_channel_ready: member add for {} in {:?} not confirmed on the relay",
                    ctx.expect_member, ctx.channel
                );
                invariant_ok = false;
            }
        } else {
            let locally = provider.with_store(|s| {
                match s.is_channel_member(ctx.channel, ctx.expect_member) {
                    Ok(present) => present,
                    Err(e) => {
                        tracing::error!(
                            channel = ctx.channel,
                            pubkey = ctx.expect_member,
                            error = %e,
                            "ensure_channel_ready: is_channel_member probe failed — treating as not-mirrored and re-syncing"
                        );
                        false
                    }
                }
            });
            if !locally {
                let role = roles
                    .get(ctx.expect_member)
                    .map(String::as_str)
                    .unwrap_or("member");
                provider.with_store(|s| {
                    if let Err(e) =
                        s.upsert_channel_member(ctx.channel, ctx.expect_member, role, now_secs())
                    {
                        tracing::error!(
                            channel = ctx.channel,
                            pubkey = ctx.expect_member,
                            error = %e,
                            "ensure_channel_ready: local member mirror sync failed — cache divergence"
                        );
                    }
                });
            }
        }

        if !invariant_ok {
            return ChannelGate::Degraded;
        }

        provider
            .readiness
            .mark_ready(ctx.channel, ctx.expect_member);
        if repaired {
            ChannelGate::Repaired
        } else {
            ChannelGate::Ready
        }
    })
}

async fn confirm_role_grant(
    provider: &Nip29Provider,
    channel: &str,
    pubkey: &str,
    want_admin: bool,
) -> bool {
    for attempt in 0..6u32 {
        let outcome = if want_admin {
            provider.nip29_add_admin_outcome(channel, pubkey).await
        } else {
            provider.nip29_add_member_outcome(channel, pubkey).await
        };
        // Confirm ONLY on a relay state we actually OBSERVED. A read-back failure
        // must never be promoted to "grant confirmed" (the old `outcome.is_applied()`
        // path did exactly that): log loud, then retry and ultimately degrade.
        match provider.try_fetch_group_state(channel).await {
            Ok((_, roles, members)) => {
                let present = if want_admin {
                    roles.get(pubkey).map(String::as_str) == Some("admin")
                } else {
                    members.contains(pubkey) || roles.contains_key(pubkey)
                };
                if present {
                    return true;
                }
            }
            Err(e) => {
                tracing::error!(
                    channel,
                    pubkey,
                    attempt,
                    error = %e,
                    "confirm_role_grant: relay read-back failed; cannot confirm grant — retrying then degrading"
                );
            }
        }
        if outcome.is_rejected() {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt as u64 + 1))).await;
    }
    false
}
