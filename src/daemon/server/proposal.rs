use super::*;

#[derive(serde::Deserialize, Default)]
#[allow(dead_code)]
pub(in crate::daemon::server) struct ProposeParams {
    title: String,
    body: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default, alias = "env_session")]
    harness_session: Option<String>,
    #[serde(default)]
    pty_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    group: Option<String>,
    /// Stable `d` identifier. When Some, the kind:30023 supersedes any prior
    /// proposal with the same (author, d) — a revision. When None, mint one.
    #[serde(default)]
    d: Option<String>,
}

/// Publish a kind:30023 (NIP-23 long-form) proposal signed by the agent's identity.
///
/// Tags:
///   ["d", <short-id>]           — addressable identifier (NIP-33)
///   ["title", <title>]          — human-readable title
///   ["h", <channel>]            — NIP-29 group
///   ["p", <owner>]              — per owner in cfg.owners, surfaces to the human
///   (no agent/session tag — author identity is the event signer pubkey; kind:0 carries slug)
pub(in crate::daemon::server) async fn rpc_propose(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ProposeParams =
        serde_json::from_value(params.clone()).context("parsing propose params")?;
    if p.title.is_empty() {
        anyhow::bail!("title must not be empty");
    }

    // Resolve session if one is live; fall back to cwd-based channel + env agent.
    // propose doesn't require a live session — it just needs a channel and a key.
    let session_rec = resolve_session(state, &CallerAnchor::from_params(params)).ok();
    let cwd = p
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    // Publish into the session's CURRENT routing scope — channel when set
    // (a `channels switch` moved it to a subgroup), else the per-session room.
    // Falls back to the cwd-derived channel when no session is live (a bare
    // `tenex-edge publish` from the repo root).
    let channel = session_rec
        .as_ref()
        .map(|r| r.channel_h.clone())
        .unwrap_or_else(|| crate::workspace::resolve(&cwd).unwrap_or_default());
    let agent_slug = session_rec
        .as_ref()
        .map(|r| r.agent_slug.clone())
        .or_else(|| p.agent.clone().filter(|a| !a.is_empty()))
        .unwrap_or_else(|| "agent".to_string());
    let id = identity::load_or_create(&config::edge_home(), &agent_slug, now_secs())?;

    // Addressable `d` identifier. A caller-supplied `d` makes this a REVISION
    // that supersedes the prior (author, d) at the same naddr; otherwise mint one.
    let d_tag = p.d.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        format!(
            "prop-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        )
    });

    // Build the Proposal domain event; the wire shape lives in the NIP-29 provider.
    let ev = DomainEvent::Proposal(crate::domain::Proposal {
        agent: crate::domain::AgentRef::new(id.pubkey_hex(), agent_slug.clone()),
        channel: channel.clone(),
        title: p.title.clone(),
        body: p.body.clone(),
        d: d_tag.clone(),
        // Surface to each owner.
        audience: state.owners.clone(),
    });
    // Checked publish: a NIP-29 relay rejecting the kind:30023 (e.g. the author
    // isn't a member of the channel group) used to resolve Ok and report a false
    // "published" — silent data loss. `publish_checked` fails on relay rejection
    // so the CLI exits nonzero with the relay's stated reason.
    // Sign with the session's own minted key when a live session is present; else
    // the durable local agent key (a bare `publish` from the repo root).
    let proposal_signing_keys = match session_rec.as_ref() {
        Some(r) => state.session_signing_keys(&r.session_id)?,
        None => id.keys.clone(),
    };
    let event_id = state
        .provider
        .publish_checked(&ev, &proposal_signing_keys)
        .await
        .context("publishing proposal")?;
    let eid_hex = event_id.to_hex();

    // Internal read-back: confirm the event is actually retrievable from the
    // relay, not merely accepted. Surfaces a relay that ACKs writes but silently
    // drops them. Best-effort and non-fatal — reported to the caller so it can
    // warn loudly without failing a publish the relay genuinely accepted.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let retrievable = state
        .provider
        .is_retrievable(event_id, Duration::from_secs(5))
        .await;

    Ok(serde_json::json!({
        "event_id": eid_hex,
        "d_tag": d_tag,
        "title": p.title,
        "retrievable": retrievable,
    }))
}
