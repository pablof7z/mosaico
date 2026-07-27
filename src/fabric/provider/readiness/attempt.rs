use super::{ChannelCtx, ChannelGate, Nip29Provider};
use crate::fabric::nip29::readiness::ChannelReadinessError;

pub(super) fn degraded(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    reason: impl Into<String>,
) -> ChannelGate {
    degraded_error(provider, ctx, ChannelReadinessError::reason(reason))
}

pub(super) fn degraded_error(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    error: ChannelReadinessError,
) -> ChannelGate {
    record(provider, ctx, "degraded", error.to_string());
    ChannelGate::Degraded(error)
}

pub(super) fn finish(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    gate: ChannelGate,
    reason: impl Into<String>,
) -> ChannelGate {
    let outcome = match &gate {
        ChannelGate::Ready => "ready",
        ChannelGate::Repaired => "repaired",
        ChannelGate::Degraded(_) => "degraded",
    };
    record(provider, ctx, outcome, reason);
    gate
}

pub(super) fn record(
    provider: &Nip29Provider,
    ctx: &ChannelCtx<'_>,
    outcome: &str,
    reason: impl Into<String>,
) {
    provider.with_store(|s| {
        let _ = s.record_channel_readiness_attempt(&crate::state::NewChannelReadinessAttempt {
            channel_h: ctx.channel.to_string(),
            expect_member: ctx.expect_member.to_string(),
            parent_hint: ctx.parent_hint.map(str::to_string),
            name: ctx.name.map(str::to_string),
            source: "provider.ensure_channel_ready".to_string(),
            outcome: outcome.to_string(),
            reason: reason.into(),
            created_at: crate::util::now_secs(),
        });
    });
}
