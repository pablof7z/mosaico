use super::*;

/// Route one daemon RPC request to its handler. The single authority for
/// method dispatch; every method name lives here so the protocol surface is
/// auditable in one place. Failures are logged server-side before being
/// returned, since the error frame is the only durable record of a failed call.
pub(super) async fn dispatch(state: &Arc<DaemonState>, req: &Request) -> Response {
    let result = match req.method.as_str() {
        "ping" => Ok(serde_json::json!({"pong": true})),
        #[cfg(feature = "stress-harness")]
        "stress_nmp_snapshot" => Ok(state.nmp().stress_snapshot()),
        "shutdown" => {
            state.connections.shutdown.notify_waiters();
            Ok(serde_json::json!({"stopped": true}))
        }
        "who" => rpc_who(state, &req.params),
        "my_session" => rpc_my_session(state, &req.params),
        "my_session_status" => rpc_my_session_status(state, &req.params).await,
        "mcp_actor_resolve" => mcp_actor::rpc_resolve(state, &req.params).await,
        "session_start" => rpc_session_start(state, &req.params, None).await,
        "session_end" => rpc_session_end(state, &req.params).await,
        "session_kill" => rpc_session_kill(state, &req.params).await,
        "session_pty_wrap" => rpc_session_pty_wrap(state, &req.params).await,
        "session_delivery_wait" => session_delivery::rpc_wait(state, &req.params).await,
        "session_delivery_ack" => session_delivery::rpc_ack(state, &req.params).await,
        "pi_tool_call" => pi_tools::rpc_call(state, &req.params).await,
        "channel_send" => rpc_channel_send(state, &req.params).await,
        "channel_search" => channel_search::rpc_channel_search(state, &req.params),
        "channel_wait" => channel_wait::rpc_channel_wait(state, &req.params).await,
        "channel_reply" => channel_send::rpc_channel_reply(state, &req.params).await,
        "channel_react" => channel_send::rpc_channel_react(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params).await,
        "turn_end" => rpc_turn_end(state, &req.params).await,
        "cross_project_path_classify" => cross_project_boundary::rpc_classify(state, &req.params),
        "explain" => rpc_explain(state, &req.params),
        "local_backend" => rpc_local_backend(state),
        "root_channels" => rpc::rpc_root_channels(state),
        "channel_members" => rpc::rpc_channel_members(state, &req.params).await,
        "channel_add_member" => rpc::rpc_channel_add_member(state, &req.params).await,
        "channel_remove_member" => rpc::rpc_channel_remove_member(state, &req.params).await,
        "operator_sessions" => operator_sessions::rpc_operator_sessions(state),
        "agent_inventory" => agent_discovery::rpc_agent_inventory(state, &req.params),
        "agent_save" => agent_config::rpc_agent_save(&state.agent_config, &req.params),
        "agent_key_status" => agent_config::rpc_agent_key_status(&state.agent_config, &req.params),
        "agent_key_create" => agent_config::rpc_agent_key_create(&state.agent_config, &req.params),
        "agent_remove" => agent_config::rpc_agent_remove(&state.agent_config, &req.params),
        "agent_usage" => agent_usage::rpc_agent_usage(state, &req.params),
        "pty_supervisor_exit" => rpc::rpc_pty_supervisor_exit(state, &req.params).await,
        "pty_presentation_changed" => {
            managed_lifecycle::rpc_pty_presentation_changed(state, &req.params)
        }
        "backend_profile_refresh" => rpc_backend_profile_refresh(state),
        "channel_create" => rpc_channel_create(state, &req.params).await,
        "channel_init" => channel_init::rpc_channel_init(state, &req.params).await,
        "channel_edit" => rpc_channel_edit(state, &req.params).await,
        "channel_resolve" => rpc_channel_resolve(state, &req.params).await,
        "channel_list" => rpc_channel_list(state, &req.params),
        "channel_archive" => rpc_channel_archive(state, &req.params).await,
        "channel_delete" => rpc_channel_delete(state, &req.params).await,
        "channel_join" => rpc_channel_join(state, &req.params).await,
        "channel_leave" => rpc_channel_leave(state, &req.params).await,
        "channel_move_accept" => channel_move::rpc_accept(state, &req.params).await,
        "dispatch" => session_dispatch::rpc_dispatch(state, &req.params).await,
        "pty_status" => pty_rpc::rpc_pty_status(state).await,
        "pty_send" => pty_rpc::rpc_pty_send(state, &req.params).await,
        "pty_spawn" => pty_rpc::rpc_pty_spawn(state, &req.params).await,
        "pty_launch_existing" => pty_rpc::rpc_pty_launch_existing(state, &req.params).await,
        "invite" => invite_rpc::rpc_invite(state, &req.params).await,
        "pty_attach" => pty_rpc::rpc_pty_attach(state, &req.params),
        "pty_resume" => pty_rpc::rpc_pty_resume(state, &req.params).await,
        "pty_resume_native" => pty_rpc::rpc_pty_resume_native(state, &req.params).await,
        "pty_resumable" => pty_rpc::rpc_pty_resumable(state).await,
        other => Err(anyhow::anyhow!("unknown method {other}")),
    };
    match result {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => {
            // RPC failures reach the caller as `Response::err`, but that frame was the only record:
            // nothing server-side recorded that a call failed
            // or why, making a hook-path failure (which itself only logs to a
            // stderr no one durably captures) unrecoverable after the fact.
            tracing::error!(method = %req.method, error = %format!("{e:#}"), "rpc call failed");
            Response::err(req.id, "rpc_error", format!("{e:#}"))
        }
    }
}
