//! Resolve a `channel add --session` selector to a concrete session. Selectors
//! arrive as a raw opaque session id, or — the recruiting-facing form — an
//! `@agent/session` handle. Legacy `codename@host` selectors are still accepted
//! by scanning live/known sessions for the old friendly code; they are not the
//! current public handle model.

use crate::daemon::server::DaemonState;
use crate::state::Session;
use anyhow::Result;
use std::sync::Arc;

pub(super) struct RemoteSession {
    pub(super) session_id: String,
    pub(super) pubkey: String,
    pub(super) slug: String,
    pub(super) backend: String,
}

/// Split a legacy selector into `(code_or_id, host?)`. A leading `@` sigil is
/// optional; `code@host` splits on the last `@`. A bare token carries no host.
fn split_legacy_code_host(selector: &str) -> (String, Option<String>) {
    let s = selector.trim();
    let s = s.strip_prefix('@').unwrap_or(s);
    match s.rsplit_once('@') {
        Some((code, host)) => (code.to_string(), Some(host.to_string())),
        None => (s.to_string(), None),
    }
}

/// A LOCAL session for the selector: public `agent/session`, then raw id/prefix,
/// then the legacy friendly-code scan. A selector that names a non-local host is
/// never matched here.
pub(super) fn local_session(state: &Arc<DaemonState>, selector: &str) -> Option<Session> {
    if let Some(rec) = local_session_by_public_handle(state, selector) {
        return Some(rec);
    }
    if let Some(rec) = state
        .with_store(|s| s.get_session(selector))
        .ok()
        .flatten()
        .or_else(|| {
            state
                .with_store(|s| s.find_session_by_prefix(selector))
                .ok()
                .flatten()
        })
    {
        return Some(rec);
    }
    let (legacy_code, host) = split_legacy_code_host(selector);
    // A host that names another backend means the caller wants a remote session.
    if host
        .as_deref()
        .is_some_and(|h| !h.is_empty() && h != state.host)
    {
        return None;
    }
    state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .find(|rec| crate::util::friendly_short_code(&rec.session_id) == legacy_code)
    })
}

fn local_session_by_public_handle(state: &Arc<DaemonState>, selector: &str) -> Option<Session> {
    let selector = selector.trim().strip_prefix('@').unwrap_or(selector.trim());
    let (agent, session_ref) = crate::idref::parse_session_handle(selector)?;
    state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .find(|rec| {
                if rec.agent_slug != agent {
                    return false;
                }
                rec.session_id == session_ref
                    || rec.session_id.starts_with(session_ref)
                    || crate::util::friendly_short_code(&rec.session_id) == session_ref
            })
    })
}

/// A REMOTE session for the selector, from the materialized status cache. Matches
/// `@agent/session`, raw id (exact/prefix), or a legacy code@host handle.
pub(super) fn remote_session_from_status(
    state: &Arc<DaemonState>,
    selector: &str,
) -> Result<RemoteSession> {
    if let Some((agent, session_ref)) = crate::idref::parse_session_handle(
        selector.trim().strip_prefix('@').unwrap_or(selector.trim()),
    ) {
        return remote_session_from_public_handle(state, selector, agent, session_ref);
    }
    let (legacy_code, want_host) = split_legacy_code_host(selector);
    let matches = state.with_store(|s| -> Result<Vec<RemoteSession>> {
        let mut out = Vec::new();
        for st in s.list_status_sessions(None, None)? {
            let by_id = st.session_id == selector || st.session_id.starts_with(selector);
            let by_code = crate::util::friendly_short_code(&st.session_id) == legacy_code;
            if !by_id && !by_code {
                continue;
            }
            let Some(profile) = s.get_profile(&st.pubkey)? else {
                continue;
            };
            // When resolving a legacy code with an explicit host, only that backend's
            // session qualifies.
            if by_code && !by_id {
                if let Some(h) = want_host.as_deref().filter(|h| !h.is_empty()) {
                    if profile.host != h {
                        continue;
                    }
                }
            }
            let slug = if profile.slug.is_empty() {
                st.slug.clone()
            } else {
                profile.slug.clone()
            };
            out.push(RemoteSession {
                session_id: st.session_id,
                pubkey: st.pubkey,
                slug,
                backend: profile.host,
            });
        }
        out.sort_by(|a, b| {
            a.session_id
                .cmp(&b.session_id)
                .then(a.pubkey.cmp(&b.pubkey))
        });
        out.dedup_by(|a, b| a.session_id == b.session_id && a.pubkey == b.pubkey);
        Ok(out)
    })?;
    match matches.as_slice() {
        [one] => Ok(RemoteSession {
            session_id: one.session_id.clone(),
            pubkey: one.pubkey.clone(),
            slug: one.slug.clone(),
            backend: one.backend.clone(),
        }),
        [] => anyhow::bail!("no session matching {selector:?}"),
        _ => anyhow::bail!(
            "session {selector:?} is ambiguous; use the full session id or @agent/session"
        ),
    }
}

fn remote_session_from_public_handle(
    state: &Arc<DaemonState>,
    selector: &str,
    agent: &str,
    session_ref: &str,
) -> Result<RemoteSession> {
    let matches = state.with_store(|s| -> Result<Vec<RemoteSession>> {
        let mut out = Vec::new();
        for st in s.list_status_sessions(None, None)? {
            let profile = s.get_profile(&st.pubkey)?;
            let profile_agent = profile
                .as_ref()
                .map(|p| p.agent_slug.as_str())
                .filter(|slug| !slug.is_empty())
                .or_else(|| crate::idref::parse_session_handle(&st.slug).map(|(slug, _)| slug));
            if profile_agent != Some(agent) {
                continue;
            }
            let by_session = st.session_id == session_ref
                || st.session_id.starts_with(session_ref)
                || crate::util::friendly_short_code(&st.session_id) == session_ref;
            if !by_session {
                continue;
            }
            let backend = profile.as_ref().map(|p| p.host.clone()).unwrap_or_default();
            let slug = crate::idref::session_handle(
                agent,
                &crate::util::friendly_short_code(&st.session_id),
            );
            out.push(RemoteSession {
                session_id: st.session_id,
                pubkey: st.pubkey,
                slug,
                backend,
            });
        }
        out.sort_by(|a, b| {
            a.session_id
                .cmp(&b.session_id)
                .then(a.pubkey.cmp(&b.pubkey))
        });
        out.dedup_by(|a, b| a.session_id == b.session_id && a.pubkey == b.pubkey);
        Ok(out)
    })?;
    match matches.as_slice() {
        [one] => Ok(RemoteSession {
            session_id: one.session_id.clone(),
            pubkey: one.pubkey.clone(),
            slug: one.slug.clone(),
            backend: one.backend.clone(),
        }),
        [] => anyhow::bail!("no session matching {selector:?}"),
        _ => anyhow::bail!("session {selector:?} is ambiguous; use the full agent/session handle"),
    }
}
