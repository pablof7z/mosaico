use super::Nip29Provider;
use crate::fabric::group_management::GroupMutationOutcome;
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate, ChannelReadinessError};
use std::future::Future;
use std::pin::Pin;

mod admins;
mod ancestry;
mod attempt;
mod local;
mod provision;
mod verify;

impl Nip29Provider {
    /// Ensure `ctx.channel` exists on the relay and has `ctx.expect_member`.
    pub(crate) async fn ensure_channel_ready<'a>(&'a self, ctx: ChannelCtx<'a>) -> ChannelGate {
        // Never provision an empty channel id: a 9007 create-group with an empty
        // `h`/`d` mints a junk relay group (kind:39000 with d=""). An empty
        // scope means "no channel resolved",
        // which is a caller bug, not a group to create — fail closed.
        if ctx.channel.trim().is_empty() {
            eprintln!("[daemon] ensure_channel_ready: refusing to provision an empty channel id");
            attempt::record(self, &ctx, "degraded", "empty channel id");
            return attempt::degraded(self, &ctx, "empty channel id");
        }
        let parent_hint =
            match ancestry::resolved_parent_hint(self, ctx.channel, ctx.parent_hint).await {
                Ok(parent) => parent,
                Err(error) => {
                    return attempt::degraded(
                        self,
                        &ctx,
                        format!("resolving channel ancestry failed: {error:#}"),
                    );
                }
            };
        let normalized = ChannelCtx {
            channel: ctx.channel,
            expect_member: ctx.expect_member,
            parent_hint: parent_hint.as_deref(),
            name: ctx.name,
        };
        ensure_channel_ready_inner(self, normalized).await
    }
}

fn ensure_channel_ready_inner<'a>(
    provider: &'a Nip29Provider,
    ctx: ChannelCtx<'a>,
) -> Pin<Box<dyn Future<Output = ChannelGate> + Send + 'a>> {
    Box::pin(async move {
        // No depth cap: a channel path may nest arbitrarily deep (mkdir -p style),
        // so provisioning walks the whole ancestor chain up to the channel root.
        // Parent links are a strict acyclic ancestry delivered by NMP,
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
        if local::is_ready(provider, &ctx).await {
            provider
                .readiness
                .mark_ready(ctx.channel, ctx.expect_member);
            return attempt::finish(
                provider,
                &ctx,
                ChannelGate::Ready,
                "channel readiness verified from NMP group state",
            );
        }

        let Some(mgmt_keys) = provider.management_keys() else {
            return attempt::degraded(provider, &ctx, "management signing key unavailable");
        };
        let mgmt_pubkey = mgmt_keys.public_key().to_hex();

        let parent_admins: Vec<String> = if let Some(parent) = parent_hint {
            match ancestry::ensure_parent(provider, parent, &mgmt_pubkey).await {
                Ok(admins) => admins,
                Err(error) => {
                    return attempt::degraded_error(provider, &ctx, error);
                }
            }
        } else {
            vec![]
        };
        // A locally reserved root/child id is exact product intent. If its
        // retained NMP observation has no records yet, send the one create
        // operation and let NMP's terminal result decide it; do not open a
        // second observation merely to prove absence. For an unrelated id,
        // absence is not ours to invent, so the bounded NMP read still fails
        // closed.
        let managed = provider
            .with_store(|store| store.is_managed_channel(ctx.channel))
            .unwrap_or(false);
        let existing_snapshot = match provider.current_group_snapshot(ctx.channel) {
            Some(snapshot) => Some(snapshot),
            None if managed => None,
            None => match provider.group_snapshot(ctx.channel).await {
                Ok(snapshot) => Some(snapshot),
                Err(e) => {
                    tracing::error!(
                        channel = ctx.channel,
                        error = %format!("{e:#}"),
                        "ensure_channel_ready: relay fetch failed — degrading without attempting creation (no fabrication-by-omission)"
                    );
                    return attempt::degraded(provider, &ctx, format!("relay fetch failed: {e:#}"));
                }
            },
        };
        let group_exists = existing_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.metadata.is_some());
        let management_is_admin = existing_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .admins
                .iter()
                .any(|admin| admin.pubkey.to_hex() == mgmt_pubkey)
        });
        let mut repaired = false;
        let mut admins_published_this_attempt = Vec::new();
        if !group_exists {
            match provision::missing_group(provider, &ctx, parent_hint).await {
                Ok(true) => {
                    if let Err(error) = verify::initialize_created_group(
                        provider,
                        &ctx,
                        &mgmt_pubkey,
                        &parent_admins,
                    )
                    .await
                    {
                        return attempt::degraded_error(provider, &ctx, error);
                    }
                    provider
                        .readiness
                        .mark_ready(ctx.channel, ctx.expect_member);
                    return attempt::finish(
                        provider,
                        &ctx,
                        ChannelGate::Repaired,
                        "channel creation and required role publications settled through NMP",
                    );
                }
                Ok(false) => {}
                Err(error) => return attempt::degraded_error(provider, &ctx, error),
            }
        } else if !management_is_admin {
            let required_admins = verify::required_admins(provider, &mgmt_pubkey, &parent_admins);
            let granted = provider
                .try_grant_admins_via_user_nsec(ctx.channel, &required_admins)
                .await;
            match granted {
                GroupMutationOutcome::Published => {}
                GroupMutationOutcome::Failed(error) => {
                    return attempt::degraded_error(
                        provider,
                        &ctx,
                        ChannelReadinessError::from(error).context("management self-grant failed"),
                    );
                }
            }
            // No optimistic local patch is needed: the relay accepted the
            // mutation, and the retained group observation owns roster state.
            admins_published_this_attempt = required_admins;
            repaired = true;
        }

        // A subgroup is not ready merely because its own metadata and roster are
        // healthy: the parent must reciprocally list it (NIP-29 parent consent).
        // Use the relay-declared parent rather than the caller's soft hint, then
        // require the relay-owned reverse projection before opening the gate.
        let declared_parent = match provider.try_fetch_group_parent(ctx.channel).await {
            Ok(parent) => parent,
            Err(e) => {
                tracing::error!(
                    channel = ctx.channel,
                    error = %format!("{e:#}"),
                    "ensure_channel_ready: could not verify subgroup parent metadata"
                );
                return attempt::degraded(
                    provider,
                    &ctx,
                    format!("subgroup parent metadata fetch failed: {e:#}"),
                );
            }
        };
        if let Some(parent) = declared_parent {
            if parent == ctx.channel {
                return attempt::degraded(
                    provider,
                    &ctx,
                    "relay metadata declares the channel as its own parent",
                );
            }
            match provider
                .confirm_parent_lists_child(&parent, ctx.channel)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    tracing::error!(
                        channel = ctx.channel,
                        parent,
                        error = %format!("{e:#}"),
                        "ensure_channel_ready: reciprocal parent child relationship was not confirmed"
                    );
                    return attempt::degraded(
                        provider,
                        &ctx,
                        format!("reciprocal parent child relationship failed: {e:#}"),
                    );
                }
            }
        }

        // NMP's group snapshot is the authority. Read it directly; readiness
        // never depends on a Mosaico-owned channel copy.
        match provider.fetch_channel(ctx.channel).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return attempt::degraded(
                    provider,
                    &ctx,
                    "settled NMP group snapshot contained no kind:39000 metadata",
                );
            }
            Err(error) => {
                return attempt::degraded(
                    provider,
                    &ctx,
                    format!("group metadata acquisition failed: {error:#}"),
                );
            }
        }

        let invariant = verify::ensure_invariants(
            provider,
            &ctx,
            &mgmt_pubkey,
            &parent_admins,
            &admins_published_this_attempt,
        )
        .await;
        if let Some(error) = invariant.degraded {
            return attempt::degraded_error(provider, &ctx, error);
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

#[cfg(test)]
mod tests;
