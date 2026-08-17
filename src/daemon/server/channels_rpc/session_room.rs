use super::*;

pub(in crate::daemon::server) async fn ensure_session_room(
    state: &Arc<DaemonState>,
    room_h: &str,
    name: &str,
    parent: &str,
    member_pubkey: &str,
) -> crate::fabric::nip29::readiness::ChannelGate {
    let reserved = state.with_store(|store| {
        store.reserve_channel_resolution_intent(parent, name, room_h, crate::util::now_secs())
    });
    match reserved {
        Ok(channel) if channel == room_h => {}
        Ok(channel) => {
            return degraded(format!(
                "session room {name:?} is already reserved as {channel}"
            ));
        }
        Err(error) => {
            return degraded(format!("reserving session room {name:?} failed: {error:#}"));
        }
    }
    let gate = state
        .snapshot()
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: room_h,
            expect_member: member_pubkey,
            parent_hint: Some(parent),
            name: Some(name),
        })
        .await;
    let _ = ensure_subscription(state, room_h).await;
    gate
}

fn degraded(reason: String) -> crate::fabric::nip29::readiness::ChannelGate {
    crate::fabric::nip29::readiness::ChannelGate::Degraded(
        crate::fabric::nip29::readiness::ChannelReadinessError::reason(reason),
    )
}
