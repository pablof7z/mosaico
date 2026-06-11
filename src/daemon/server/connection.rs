use super::admin::{rpc_acl, rpc_doctor, rpc_project_edit, rpc_project_list};
use super::awareness::rpc_who;
use super::inbox::{
    fetch_mentions_into_inbox, rows_to_json, rpc_inbox, rpc_turn_check, rpc_turn_end,
    rpc_turn_start, rpc_user_prompt,
};
use super::lifecycle::ensure_subscription;
use super::messaging::{rpc_propose, rpc_send_message};
use super::session::{resolve_session, rpc_session_end, rpc_session_start};
use super::*;

// ── connection handling ──────────────────────────────────────────────────────

pub(super) async fn serve_connection(state: Arc<DaemonState>, stream: UnixStream) -> Result<()> {
    let (rh, wh) = stream.into_split();
    let mut reader = BufReader::new(rh);
    let mut writer = wh;

    let mut first = String::new();
    if reader.read_line(&mut first).await? == 0 {
        return Ok(());
    }
    let hello: Hello = serde_json::from_str(first.trim_end()).context("parsing hello")?;
    write_json(
        &mut writer,
        &Welcome {
            protocol: protocol_version(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .await?;

    if hello.protocol > protocol_version() {
        let mut line = String::new();
        if reader.read_line(&mut line).await? > 0
            && serde_json::from_str::<PleaseExit>(line.trim_end()).is_ok()
        {
            eprintln!(
                "[daemon] newer client (protocol {}); exiting for re-exec",
                hello.protocol
            );
            state.shutdown.notify_waiters();
        }
        let _ = write_json(
            &mut writer,
            &Response::err(0, ERR_PROTOCOL_SKEW, "daemon exiting for re-exec"),
        )
        .await;
        return Ok(());
    }

    {
        *state.open_clients.lock().unwrap() += 1;
        state.liveness_changed.notify_waiters();
    }
    let _guard = ClientGuard(state.clone());

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                write_json(&mut writer, &Response::err(0, "bad_request", e.to_string())).await?;
                continue;
            }
        };
        match req.method.as_str() {
            "tail" => {
                handle_tail(&state, req.id, &req.params, &mut writer).await?;
                break; // tail owns the connection until the client disconnects
            }
            "wait_for_mention" => {
                let resp = handle_wait_for_mention(&state, &req).await;
                write_json(&mut writer, &resp).await?;
            }
            _ => {
                let resp = dispatch(&state, &req).await;
                write_json(&mut writer, &resp).await?;
            }
        }
    }
    Ok(())
}

struct ClientGuard(Arc<DaemonState>);
impl Drop for ClientGuard {
    fn drop(&mut self) {
        let mut n = self.0.open_clients.lock().unwrap();
        *n = n.saturating_sub(1);
        self.0.liveness_changed.notify_waiters();
    }
}

async fn write_json<T: serde::Serialize, W: AsyncWriteExt + Unpin>(w: &mut W, v: &T) -> Result<()> {
    let mut line = serde_json::to_string(v)?;
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

// ── dispatch (one-shot verbs) ────────────────────────────────────────────────

async fn dispatch(state: &Arc<DaemonState>, req: &Request) -> Response {
    let result = match req.method.as_str() {
        "ping" => Ok(serde_json::json!({"pong": true})),
        "who" => rpc_who(state, &req.params),
        "session_start" => rpc_session_start(state, &req.params).await,
        "session_end" => rpc_session_end(state, &req.params),
        "send_message" => rpc_send_message(state, &req.params).await,
        "propose" => rpc_propose(state, &req.params).await,
        "inbox" => rpc_inbox(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params),
        "turn_end" => rpc_turn_end(state, &req.params).await,
        "acl" => rpc_acl(state, &req.params).await,
        "doctor" => rpc_doctor(state).await,
        "user_prompt" => rpc_user_prompt(state, &req.params).await,
        "project_list" => rpc_project_list(state).await,
        "project_edit" => rpc_project_edit(state, &req.params).await,
        other => Err(anyhow::anyhow!("unknown method {other}")),
    };
    match result {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => Response::err(req.id, "rpc_error", format!("{e:#}")),
    }
}

// ── wait_for_mention (long-poll) ─────────────────────────────────────────────

async fn handle_wait_for_mention(state: &Arc<DaemonState>, req: &Request) -> Response {
    #[derive(serde::Deserialize, Default)]
    struct P {
        #[serde(default)]
        session: Option<String>,
        #[serde(default)]
        env_session: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default = "default_timeout")]
        timeout: u64,
        #[serde(default)]
        agent: Option<String>,
    }
    fn default_timeout() -> u64 {
        300
    }
    let p: P = serde_json::from_value(req.params.clone()).unwrap_or_default();
    let rec = match resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => return Response::err(req.id, "rpc_error", format!("{e:#}")),
    };
    let _ = fetch_mentions_into_inbox(state, &rec).await;

    let deadline = if p.timeout > 0 {
        Some(tokio::time::Instant::now() + Duration::from_secs(p.timeout))
    } else {
        None
    };

    loop {
        let rows = state.with_store(|s| {
            let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
            for r in &rows {
                s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                    .ok();
            }
            rows
        });
        if !rows.is_empty() {
            let rows_json = state.with_store(|s| rows_to_json(s, &rows));
            return Response::ok(req.id, serde_json::json!({ "rows": rows_json }));
        }
        // Park until a mention is routed or a short timeout for re-check.
        let wait = state.mention_notify.notified();
        let timed_out = match deadline {
            Some(d) => {
                let now = tokio::time::Instant::now();
                if now >= d {
                    true
                } else {
                    tokio::select! {
                        _ = wait => false,
                        _ = tokio::time::sleep_until(d.min(now + Duration::from_millis(500))) => {
                            tokio::time::Instant::now() >= d
                        }
                    }
                }
            }
            None => {
                wait.await;
                false
            }
        };
        if timed_out {
            return Response::ok(req.id, serde_json::json!({ "rows": [] }));
        }
    }
}

// ── tail (streaming) ──────────────────────────────────────────────────────────

async fn handle_tail<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let project = params
        .get("project")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // Ensure the requested project is in the union subscription so its events
    // flow through the shared connection.
    if let Some(pr) = &project {
        let _ = ensure_subscription(state, pr).await;
    }
    let mut rx = state.tail_subscribe();
    {
        *state.open_clients.lock().unwrap() += 1;
        state.liveness_changed.notify_waiters();
    }
    let _guard = ClientGuard(state.clone());

    loop {
        match rx.recv().await {
            Ok(de) => {
                if let Some(line) = render_fabric_line(&de, project.as_deref()) {
                    if write_json(
                        writer,
                        &Response::item(id, serde_json::json!({ "line": line })),
                    )
                    .await
                    .is_err()
                    {
                        break; // client disconnected
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
        }
    }
    let _ = write_json(writer, &Response::end(id)).await;
    Ok(())
}

/// Render a fabric event for `tail`, scoped to `project` if given. Mirrors the
/// old CLI `render()` output so `tail` looks identical.
fn render_fabric_line(de: &DomainEvent, project: Option<&str>) -> Option<String> {
    if let Some(pr) = project {
        let matches = match de {
            DomainEvent::Presence(p) => p.project == pr,
            DomainEvent::Activity(a) => a.project == pr,
            DomainEvent::Status(s) => s.project == pr,
            DomainEvent::Mention(m) => m.project == pr,
            DomainEvent::TurnReply(r) => r.project == pr,
            DomainEvent::Profile(_) => true,
        };
        if !matches {
            return None;
        }
    }
    Some(crate::cli::render_fabric(de))
}
