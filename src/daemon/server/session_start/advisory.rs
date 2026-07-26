//! Pure session-start planning from already-observed host state.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionStartRequest {
    pub pubkey: String,
    pub channel_h: String,
    pub rel_cwd: String,
    pub room_parent: Option<String>,
    pub readiness_parent: Option<String>,
    pub channel_provision_name: Option<String>,
    pub watch_pid: Option<i32>,
    pub pty_session: Option<String>,
    pub ring_doorbell: bool,
    pub already_running: bool,
    pub channel_already_subscribed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChannelReadyIntent {
    pub channel_h: String,
    pub room_parent: Option<String>,
    pub readiness_parent: Option<String>,
    pub name: Option<String>,
    pub pubkey: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EngineStartIntent {
    pub rel_cwd: String,
    pub watch_pid: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionStartPlan {
    pub channel_ready: Option<ChannelReadyIntent>,
    pub ring_doorbell: bool,
    pub replay_chat: bool,
    pub spawn: Option<EngineStartIntent>,
    pub emit_tail: bool,
    pub reassert: bool,
}

pub(super) fn plan(request: &SessionStartRequest) -> SessionStartPlan {
    let active = !request.already_running;
    let scoped = !request.channel_h.is_empty();
    SessionStartPlan {
        channel_ready: (active && scoped).then(|| ChannelReadyIntent {
            channel_h: request.channel_h.clone(),
            room_parent: request.room_parent.clone(),
            readiness_parent: request.readiness_parent.clone(),
            name: request.channel_provision_name.clone(),
            pubkey: request.pubkey.clone(),
        }),
        ring_doorbell: request.ring_doorbell,
        replay_chat: active
            && scoped
            && (request.channel_already_subscribed || request.pty_session.is_some()),
        spawn: active.then(|| EngineStartIntent {
            rel_cwd: request.rel_cwd.clone(),
            watch_pid: request.watch_pid,
        }),
        emit_tail: active,
        reassert: request.already_running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(already_running: bool) -> SessionStartRequest {
        SessionStartRequest {
            pubkey: "pk".into(),
            channel_h: "room".into(),
            rel_cwd: ".".into(),
            room_parent: None,
            readiness_parent: Some("parent".into()),
            channel_provision_name: None,
            watch_pid: Some(7),
            pty_session: Some("pty".into()),
            ring_doorbell: true,
            already_running,
            channel_already_subscribed: false,
        }
    }

    #[test]
    fn active_start_plans_effects_and_reassert_does_not() {
        let active = plan(&request(false));
        assert!(active.spawn.is_some());
        assert_eq!(
            active.channel_ready.as_ref().unwrap().readiness_parent,
            Some("parent".into())
        );
        assert!(active.replay_chat);
        assert!(!active.reassert);

        let reassert = plan(&request(true));
        assert!(reassert.spawn.is_none());
        assert!(reassert.channel_ready.is_none());
        assert!(reassert.reassert);
    }

    #[test]
    fn unscoped_start_skips_channel_effects() {
        let mut unscoped = request(false);
        unscoped.channel_h.clear();

        let plan = plan(&unscoped);
        assert!(plan.channel_ready.is_none());
        assert!(!plan.replay_chat);
        assert!(plan.spawn.is_some());
    }
}
