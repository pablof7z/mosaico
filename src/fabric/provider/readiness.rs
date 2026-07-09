use super::Nip29Provider;
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate};
use std::future::Future;
use std::pin::Pin;

mod attempt;
mod verify;

impl Nip29Provider {
    /// Ensure `ctx.channel` exists on the relay and has `ctx.expect_member`.
    pub async fn ensure_channel_ready<'a>(&'a self, ctx: ChannelCtx<'a>) -> ChannelGate {
        // Never provision an empty channel id: a 9007 create-group with an empty
        // `h`/`d` mints a junk relay group (kind:39000 with d="") and a bogus
        // empty-channel_h cache row. An empty scope means "no channel resolved",
        // which is a caller bug, not a group to create — fail closed.
        if ctx.channel.trim().is_empty() {
            eprintln!("[daemon] ensure_channel_ready: refusing to provision an empty channel id");
            attempt::record(self, &ctx, "degraded", "empty channel id");
            return ChannelGate::Degraded;
        }
        ensure_channel_ready_inner(self, ctx).await
    }
}

fn ensure_channel_ready_inner<'a>(
    provider: &'a Nip29Provider,
    ctx: ChannelCtx<'a>,
) -> Pin<Box<dyn Future<Output = ChannelGate> + Send + 'a>> {
    Box::pin(async move {
        // No depth cap: a channel path may nest arbitrarily deep (mkdir -p style),
        // so provisioning walks the whole ancestor chain up to the channel root.
        // Parent links are a strict acyclic ancestry materialized from the relay,
        // so this recursion terminates at the root (parent_hint == None).

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
        if locally_materialized_ready(provider, &ctx) {
            provider
                .readiness
                .mark_ready(ctx.channel, ctx.expect_member);
            return attempt::finish(
                provider,
                &ctx,
                ChannelGate::Ready,
                "channel readiness verified from materialized relay cache",
            );
        }

        let Some(mgmt_keys) = provider.management_keys() else {
            return attempt::degraded(provider, &ctx, "management signing key unavailable");
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
            let parent_gate = ensure_channel_ready_inner(provider, parent_ctx).await;
            if matches!(parent_gate, ChannelGate::Degraded) {
                eprintln!(
                    "[daemon] ensure_channel_ready: parent {:?} is degraded; aborting for {:?}",
                    parent, ctx.channel
                );
                return attempt::degraded(
                    provider,
                    &ctx,
                    format!("parent channel {parent} readiness degraded"),
                );
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
        let (group_exists, mut roles, members) = match provider
            .try_fetch_group_state(ctx.channel)
            .await
        {
            Ok(state) => state,
            Err(e) => {
                tracing::error!(
                    channel = ctx.channel,
                    error = %format!("{e:#}"),
                    "ensure_channel_ready: relay fetch failed — degrading without attempting creation (no fabrication-by-omission)"
                );
                return attempt::degraded(provider, &ctx, format!("relay fetch failed: {e:#}"));
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
                    return attempt::degraded(
                        provider,
                        &ctx,
                        "kind:39000 did not materialize after create",
                    );
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
                return attempt::degraded(
                    provider,
                    &ctx,
                    "group creation failed and relay metadata is absent",
                );
            }
            // else: group pre-existed on the relay (create rejected because it was
            // already there); fall through to membership / admin checks.
        } else if roles.get(&mgmt_pubkey).map(String::as_str) != Some("admin") {
            let granted = provider
                .try_grant_mgmt_admin_via_user_nsec(ctx.channel, &mgmt_pubkey)
                .await;
            if !granted.is_confirmed() {
                eprintln!(
                    "[daemon] ensure_channel_ready: management key is not admin of {:?} \
                     and self-grant failed",
                    ctx.channel
                );
                return attempt::degraded(
                    provider,
                    &ctx,
                    "management key is not admin and self-grant failed",
                );
            }
            roles.insert(mgmt_pubkey.clone(), "admin".to_string());
            repaired = true;
        }

        // SOOT guarantee: a ready channel must be present in `relay_channels` from
        // the relay's OWN kind:39000 — not a local optimistic write. A freshly
        // created group was already materialized above; a pre-existing group hit by
        // a cold daemon cache is read back from the relay here (best-effort).
        if provider.with_store(|s| s.get_channel(ctx.channel).ok().flatten().is_none()) {
            provider.fetch_and_materialize_channel(ctx.channel).await;
        }

        let invariant = verify::ensure_invariants(
            provider,
            &ctx,
            &mgmt_pubkey,
            &parent_admins,
            &roles,
            &members,
        )
        .await;
        if let Some(reason) = invariant.degraded_reason {
            return attempt::degraded(provider, &ctx, reason);
        }
        repaired |= invariant.repaired;

        provider
            .readiness
            .mark_ready(ctx.channel, ctx.expect_member);
        if repaired {
            attempt::finish(
                provider,
                &ctx,
                ChannelGate::Repaired,
                "channel readiness repaired and verified",
            )
        } else {
            attempt::finish(
                provider,
                &ctx,
                ChannelGate::Ready,
                "channel readiness verified",
            )
        }
    })
}

fn locally_materialized_ready(provider: &Nip29Provider, ctx: &ChannelCtx<'_>) -> bool {
    let Some(required_admins) = local_ready_required_admins(provider, ctx) else {
        return false;
    };
    provider.with_store(|store| store_locally_materialized_ready(store, ctx, &required_admins))
}

fn local_ready_required_admins(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
) -> Option<Vec<String>> {
    let mut admins = vec![provider.management_pubkey()?];
    if ctx.repair_whitelisted_admins {
        for pk in &provider.whitelisted_pubkeys {
            if !admins.contains(pk) {
                admins.push(pk.clone());
            }
        }
    }
    Some(admins)
}

fn store_locally_materialized_ready(
    store: &crate::state::Store,
    ctx: &ChannelCtx<'_>,
    required_admins: &[String],
) -> bool {
    let channel_found = store.get_channel(ctx.channel).ok().flatten().is_some();
    if !channel_found {
        return false;
    }
    if !store
        .has_channel_membership_snapshot(ctx.channel)
        .unwrap_or(false)
    {
        return false;
    }
    let member_ready = ctx.expect_member.is_empty()
        || store
            .is_channel_member(ctx.channel, ctx.expect_member)
            .unwrap_or(false);
    let admins_ready = required_admins
        .iter()
        .all(|pk| store.is_channel_admin(ctx.channel, pk).unwrap_or(false));
    member_ready && admins_ready
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialized_relay_cache_proves_existing_member_ready() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::state::Store::open(&dir.path().join("state.db")).unwrap();
        store.upsert_channel("room", "room", "", "", 100).unwrap();
        store
            .replace_channel_admins("room", &["admin".to_string()], 101)
            .unwrap();
        store
            .replace_channel_members("room", &["member".to_string()], 102)
            .unwrap();

        let ctx = ChannelCtx {
            channel: "room",
            expect_member: "member",
            parent_hint: None,
            name: None,
            repair_whitelisted_admins: true,
        };

        assert!(store_locally_materialized_ready(
            &store,
            &ctx,
            &["admin".to_string()]
        ));
    }

    #[test]
    fn materialized_relay_cache_does_not_prove_missing_member_ready() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::state::Store::open(&dir.path().join("state.db")).unwrap();
        store.upsert_channel("room", "room", "", "", 100).unwrap();
        store
            .replace_channel_admins("room", &["admin".to_string()], 101)
            .unwrap();
        store
            .replace_channel_members("room", &["member".to_string()], 102)
            .unwrap();

        let ctx = ChannelCtx {
            channel: "room",
            expect_member: "other",
            parent_hint: None,
            name: None,
            repair_whitelisted_admins: true,
        };

        assert!(!store_locally_materialized_ready(
            &store,
            &ctx,
            &["admin".to_string()]
        ));
    }

    #[test]
    fn materialized_relay_cache_does_not_prove_missing_admin_ready() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::state::Store::open(&dir.path().join("state.db")).unwrap();
        store.upsert_channel("room", "room", "", "", 100).unwrap();
        store
            .replace_channel_admins("room", &["other-admin".to_string()], 101)
            .unwrap();
        store
            .replace_channel_members("room", &["member".to_string()], 102)
            .unwrap();

        let ctx = ChannelCtx {
            channel: "room",
            expect_member: "member",
            parent_hint: None,
            name: None,
            repair_whitelisted_admins: true,
        };

        assert!(!store_locally_materialized_ready(
            &store,
            &ctx,
            &["admin".to_string()]
        ));
    }
}
