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

        let parent_admins: Vec<String> = if let Some(parent) = ctx.parent_hint {
            let grandparent = provider.with_store(|s| s.channel_parent(parent).unwrap_or(None));
            let parent_ctx = ChannelCtx {
                channel: parent,
                expect_member: &mgmt_pubkey,
                parent_hint: grandparent.as_deref(),
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

        let (group_exists, roles, members) = provider.fetch_group_state(ctx.channel).await;
        let mut repaired = false;

        if !group_exists {
            let created = if let Some(parent) = ctx.parent_hint {
                let name = provider.with_store(|s| {
                    s.get_channel(ctx.channel)
                        .ok()
                        .flatten()
                        .map(|c| c.name)
                        .unwrap_or_default()
                });
                let name = if name.is_empty() { ctx.channel } else { &name };
                let ok = provider
                    .nip29_create_subgroup(ctx.channel, name, parent)
                    .await;
                if ok {
                    // Materialize the just-created subgroup's metadata locally; its
                    // parent is non-empty (this is a task/session channel). Ownership
                    // is relay-derived (admin membership), recorded below.
                    provider.with_store(|s| {
                        s.upsert_channel(ctx.channel, name, "", parent, now_secs())
                            .ok();
                    });
                }
                ok
            } else {
                let ok = match crate::fabric::nip29::lifecycle::group_create(ctx.channel) {
                    Ok(b) => {
                        provider
                            .publish_group_management(b, &mgmt_keys, "9007 create-group")
                            .await
                    }
                    Err(_) => false,
                };
                if ok {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let locked =
                        match crate::fabric::nip29::lifecycle::group_lock_closed(ctx.channel) {
                            Ok(b) => {
                                provider
                                    .publish_group_management(b, &mgmt_keys, "9002 lock-closed")
                                    .await
                            }
                            Err(_) => false,
                        };
                    if locked {
                        // Materialize the root channel locally. Ownership is
                        // relay-derived (admin membership materialized below).
                        provider.with_store(|s| {
                            s.upsert_channel(ctx.channel, "", "", "", now_secs()).ok();
                        });
                    }
                }
                ok
            };

            if !created {
                eprintln!(
                    "[daemon] ensure_channel_ready: failed to create {:?}",
                    ctx.channel
                );
                return ChannelGate::Degraded;
            }
            repaired = true;
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

        let mut invariant_ok = true;
        {
            let mut required_admins: Vec<String> = vec![mgmt_pubkey.clone()];
            required_admins.extend(provider.whitelisted_pubkeys.iter().cloned());
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
                        s.upsert_channel_member(ctx.channel, pk, "admin", now_secs())
                            .ok();
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
                    s.upsert_channel_member(ctx.channel, ctx.expect_member, "member", now_secs())
                        .ok();
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
                s.is_channel_member(ctx.channel, ctx.expect_member)
                    .unwrap_or(false)
            });
            if !locally {
                let role = roles
                    .get(ctx.expect_member)
                    .map(String::as_str)
                    .unwrap_or("member");
                provider.with_store(|s| {
                    s.upsert_channel_member(ctx.channel, ctx.expect_member, role, now_secs())
                        .ok();
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
        let (_, roles, members) = provider.fetch_group_state(channel).await;
        let present = if want_admin {
            roles.get(pubkey).map(String::as_str) == Some("admin")
        } else {
            members.contains(pubkey) || roles.contains_key(pubkey)
        };
        if present || (attempt > 0 && outcome.is_applied()) {
            return true;
        }
        if outcome.is_rejected() {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt as u64 + 1))).await;
    }
    false
}
