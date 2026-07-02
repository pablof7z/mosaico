use super::super::*;

#[derive(serde::Deserialize, Default)]
pub(super) struct ListSessionsParams {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    since: Option<u64>,
}

pub(in crate::daemon::server) fn rpc_agents_list_sessions(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ListSessionsParams =
        serde_json::from_value(params.clone()).context("agents_list_sessions params")?;
    let target = p
        .agent
        .as_deref()
        .and_then(crate::idref::parse_agent_backend_ref);
    let rows = state.with_store(|s| -> Result<Vec<serde_json::Value>> {
        let mut out = Vec::new();
        for st in s.list_status_sessions(None, p.since)? {
            let profile = s.get_profile(&st.pubkey).ok().flatten();
            let host = profile.as_ref().map(|p| p.host.clone()).unwrap_or_default();
            let slug = if !st.slug.is_empty() {
                st.slug.clone()
            } else {
                profile
                    .as_ref()
                    .map(|p| p.slug.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| pubkey_short(&st.pubkey))
            };
            if let Some(t) = &target {
                if t.slug != slug && t.slug != st.pubkey {
                    continue;
                }
                if let Some(backend) = &t.backend {
                    if host != *backend {
                        continue;
                    }
                }
            }
            let agent = if host.is_empty() || host == state.host {
                slug.clone()
            } else {
                format!("{slug}@{host}")
            };
            out.push(serde_json::json!({
                "channel": st.channel_h,
                "agent": agent,
                "session_id": st.session_id,
                "title": st.title,
                "last_seen": st.last_seen,
                "updated_at": st.updated_at,
                "host": host,
                "pubkey": st.pubkey,
            }));
        }
        Ok(out)
    })?;
    Ok(serde_json::json!({ "sessions": rows }))
}
