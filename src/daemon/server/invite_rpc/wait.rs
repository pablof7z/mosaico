use super::RemoteSession;
use crate::daemon::server::DaemonState;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const ONLINE_WAIT: Duration = Duration::from_secs(30);
const ONLINE_POLL: Duration = Duration::from_millis(500);

pub(super) fn live_session_ids(state: &Arc<DaemonState>) -> HashSet<String> {
    state
        .with_store(|s| s.list_alive_sessions())
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.session_id)
        .collect()
}

pub(super) fn channel_member_pubkeys(state: &Arc<DaemonState>, channel_h: &str) -> HashSet<String> {
    state
        .with_store(|s| s.list_channel_members(channel_h))
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.pubkey)
        .collect()
}

pub(super) async fn wait_local_agent_online(
    state: &Arc<DaemonState>,
    channel_h: &str,
    slug: &str,
    before: &HashSet<String>,
) -> Result<String> {
    wait_until(None, || {
        state.with_store(|s| {
            s.list_alive_sessions()
                .unwrap_or_default()
                .into_iter()
                .find(|rec| {
                    rec.agent_slug == slug
                        && !before.contains(&rec.session_id)
                        && s.is_session_joined_channel(&rec.session_id, channel_h)
                            .unwrap_or(false)
                        && s.is_channel_member(channel_h, &rec.agent_pubkey)
                            .unwrap_or(false)
                })
                .map(|rec| state.session_instance(&rec).display_slug())
        })
    })
    .await
}

pub(super) async fn wait_local_session_online(
    state: &Arc<DaemonState>,
    channel_h: &str,
    session_id: &str,
) -> Result<String> {
    wait_until(None, || {
        state.with_store(|s| {
            let rec = s.get_session(session_id).ok().flatten()?;
            let online = rec.alive
                && s.is_session_joined_channel(&rec.session_id, channel_h)
                    .unwrap_or(false)
                && s.is_channel_member(channel_h, &rec.agent_pubkey)
                    .unwrap_or(false);
            online.then(|| state.session_instance(&rec).display_slug())
        })
    })
    .await
}

pub(super) async fn wait_remote_agent_online(
    state: &Arc<DaemonState>,
    channel_h: &str,
    base_slug: &str,
    backend: &str,
    before: &HashSet<String>,
) -> Result<String> {
    wait_until(Some(backend), || {
        let members = channel_member_pubkeys(state, channel_h);
        let mut fallback = None;
        for pk in members.difference(before) {
            let label = label_for_pubkey(state, channel_h, pk, backend);
            if fallback.is_none() {
                fallback = Some(label.clone());
            }
            if slug_matches(base_slug, label.split('@').next().unwrap_or("")) {
                return Some(label);
            }
        }
        fallback
    })
    .await
}

pub(super) async fn wait_remote_session_online(
    state: &Arc<DaemonState>,
    channel_h: &str,
    remote: &RemoteSession,
) -> Result<String> {
    wait_until(Some(&remote.backend), || {
        state
            .with_store(|s| s.is_channel_member(channel_h, &remote.pubkey))
            .unwrap_or(false)
            .then(|| label_for_pubkey(state, channel_h, &remote.pubkey, &remote.backend))
    })
    .await
}

async fn wait_until(
    backend: Option<&str>,
    mut f: impl FnMut() -> Option<String>,
) -> Result<String> {
    let deadline = tokio::time::Instant::now() + ONLINE_WAIT;
    loop {
        if let Some(label) = f() {
            return Ok(label);
        }
        if tokio::time::Instant::now() >= deadline {
            if let Some(backend) = backend {
                anyhow::bail!(
                    "agent didn't come online after 30 seconds -- '{backend}' backend might be offline?"
                );
            }
            anyhow::bail!("agent didn't come online after 30 seconds");
        }
        tokio::time::sleep(ONLINE_POLL).await;
    }
}

fn label_for_pubkey(
    state: &Arc<DaemonState>,
    channel_h: &str,
    pubkey: &str,
    backend: &str,
) -> String {
    let slug = state.with_store(|s| {
        s.get_profile(pubkey)
            .ok()
            .flatten()
            .map(|p| p.slug)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                s.get_status(pubkey, "", channel_h)
                    .ok()
                    .flatten()
                    .map(|st| st.slug)
                    .filter(|s| !s.is_empty())
            })
    });
    let slug = slug.unwrap_or_else(|| crate::util::pubkey_short(pubkey));
    if backend.is_empty() || backend == state.host {
        slug
    } else {
        format!("{slug}@{backend}")
    }
}

fn slug_matches(base: &str, candidate: &str) -> bool {
    candidate == base
        || candidate
            .strip_prefix(base)
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}
