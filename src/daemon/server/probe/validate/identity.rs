//! Fabric identity validation for profile/pubkey/agent/backend targets.

use super::report::{bool_at, str_at};
use super::DaemonState;
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) struct IdentityTarget {
    kind: &'static str,
    requested: String,
}

pub(super) fn identity_target(target: &str) -> Option<IdentityTarget> {
    const PREFIXES: [(&str, &str); 10] = [
        ("profile:", "profile"),
        ("profile/", "profile"),
        ("pubkey:", "pubkey"),
        ("pubkey/", "pubkey"),
        ("identity:", "identity"),
        ("identity/", "identity"),
        ("agent:", "agent"),
        ("agent/", "agent"),
        ("backend:", "backend"),
        ("backend/", "backend"),
    ];
    PREFIXES.iter().find_map(|(prefix, kind)| {
        target
            .strip_prefix(prefix)
            .and_then(|rest| rest.split('/').next())
            .filter(|id| !id.trim().is_empty())
            .map(|requested| IdentityTarget {
                kind,
                requested: requested.to_string(),
            })
    })
}

pub(super) fn identity_evidence(
    state: &Arc<DaemonState>,
    target: &str,
    parsed: &IdentityTarget,
) -> Value {
    let host = state.host.clone();
    let result = state.with_store(|store| {
        let resolved_pubkey = match parsed.kind {
            "agent" => store.resolve_agent_pubkey(&parsed.requested, &host)?,
            "backend" => store.pubkey_for_backend_label(&parsed.requested)?,
            _ => Some(parsed.requested.clone()),
        };
        let Some(pubkey) = resolved_pubkey else {
            return Ok::<_, anyhow::Error>((None, None, None, Vec::new(), Vec::new(), Vec::new()));
        };
        let profile = store.get_profile(&pubkey)?;
        let identity = store.get_identity(&pubkey)?;
        // Per-session pubkeys are unique, so the identity itself is the only row
        // for this pubkey — there is no derivation family to enumerate.
        let derived: Vec<crate::state::Identity> = Vec::new();
        let session = match identity.as_ref().filter(|row| !row.session_id.is_empty()) {
            Some(row) => store.get_session(&row.session_id)?,
            None => None,
        };
        let member_channels = store.list_channels_where_member(&pubkey)?;
        let admin_channels = store.list_channels_where_admin(&pubkey)?;
        Ok::<_, anyhow::Error>((
            Some(pubkey),
            profile,
            identity,
            derived,
            session.into_iter().collect::<Vec<_>>(),
            vec![member_channels, admin_channels],
        ))
    });
    let (resolved_pubkey, profile, identity, derived, sessions, channel_sets) = match result {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "target": target,
                "kind": parsed.kind,
                "requested": parsed.requested,
                "supported": true,
                "found": false,
                "error": e.to_string(),
                "summary": "identity evidence could not read durable fabric ledgers",
                "reason": e.to_string(),
            });
        }
    };
    let Some(pubkey) = resolved_pubkey else {
        return json!({
            "target": target,
            "kind": parsed.kind,
            "requested": parsed.requested,
            "supported": true,
            "found": false,
            "summary": format!("{} `{}` did not resolve to a pubkey", parsed.kind, parsed.requested),
            "reason": "no profile row resolved this agent/backend label on this host",
        });
    };

    let member_channels = channel_sets.first().cloned().unwrap_or_default();
    let admin_channels = channel_sets.get(1).cloned().unwrap_or_default();
    let session = sessions.first();
    let identity_alive = identity.as_ref().is_some_and(|row| row.alive);
    let session_alive = session.is_some_and(|row| row.alive);
    let inconsistent_alive_identity = identity_alive && !session_alive;
    let found = profile.is_some()
        || identity.is_some()
        || !derived.is_empty()
        || !member_channels.is_empty()
        || !admin_channels.is_empty();

    json!({
        "target": target,
        "kind": parsed.kind,
        "requested": parsed.requested,
        "resolved_pubkey": pubkey,
        "supported": true,
        "found": found,
        "profile_found": profile.is_some(),
        "profile_name": profile.as_ref().map(|p| p.name.as_str()).unwrap_or(""),
        "profile_slug": profile.as_ref().map(|p| p.slug.as_str()).unwrap_or(""),
        "profile_host": profile.as_ref().map(|p| p.host.as_str()).unwrap_or(""),
        "profile_is_backend": profile.as_ref().map(|p| p.is_backend).unwrap_or(false),
        "profile_updated_at": profile.as_ref().map(|p| p.updated_at).unwrap_or(0),
        "identity_found": identity.is_some(),
        "identity": identity.as_ref().map(identity_json),
        "derived_identity_count": derived.len(),
        "derived_identities": derived.iter().take(5).map(identity_json).collect::<Vec<_>>(),
        "bound_session_found": session.is_some(),
        "bound_session_alive": session_alive,
        "bound_session_id": session.map(|s| s.session_id.as_str()).unwrap_or(""),
        "bound_session_channel": session.map(|s| s.channel_h.as_str()).unwrap_or(""),
        "member_channel_count": member_channels.len(),
        "admin_channel_count": admin_channels.len(),
        "member_channels": member_channels.iter().take(8).collect::<Vec<_>>(),
        "admin_channels": admin_channels.iter().take(8).collect::<Vec<_>>(),
        "inconsistent_alive_identity": inconsistent_alive_identity,
        "ok": found && !inconsistent_alive_identity,
        "summary": summary(parsed.kind, &parsed.requested, &pubkey, found, profile.is_some(), identity.is_some(), inconsistent_alive_identity),
        "reason": reason(found, profile.is_some(), identity.is_some(), inconsistent_alive_identity),
    })
}

pub(super) fn push_identity_check(
    checks: &mut Vec<Value>,
    limitations: &mut Vec<String>,
    evidence: &Value,
) {
    let status = if !str_at(evidence, "error").is_empty()
        || bool_at(evidence, "inconsistent_alive_identity")
    {
        "failed"
    } else if bool_at(evidence, "found") {
        "passed"
    } else {
        "not_proven"
    };
    checks.push(json!({
        "name": "identity",
        "status": status,
        "summary": str_at(evidence, "summary"),
    }));
    if status != "passed" && !str_at(evidence, "reason").is_empty() {
        limitations.push(str_at(evidence, "reason").to_string());
    } else if status == "passed" && !bool_at(evidence, "profile_found") {
        limitations.push("pubkey is known locally, but no relay kind:0 profile is cached".into());
    }
}

fn identity_json(row: &crate::state::Identity) -> Value {
    json!({
        "pubkey": row.pubkey,
        "agent_slug": row.agent_slug,
        "codename": row.codename,
        "session_id": row.session_id,
        "channel_h": row.channel_h,
        "native_id": row.native_id,
        "alive": row.alive,
        "created_at": row.created_at,
    })
}

fn summary(
    kind: &str,
    requested: &str,
    pubkey: &str,
    found: bool,
    profile_found: bool,
    identity_found: bool,
    inconsistent_alive_identity: bool,
) -> String {
    if inconsistent_alive_identity {
        format!("{kind} `{requested}` resolves to `{pubkey}` but has a stale alive identity")
    } else if found && profile_found && identity_found {
        format!("{kind} `{requested}` resolves to `{pubkey}` with profile and local identity")
    } else if found && profile_found {
        format!("{kind} `{requested}` resolves to `{pubkey}` with relay profile")
    } else if found {
        format!("{kind} `{requested}` resolves to `{pubkey}` from local identity/membership")
    } else {
        format!("{kind} `{requested}` has no profile, identity, or membership evidence")
    }
}

fn reason(
    found: bool,
    profile_found: bool,
    identity_found: bool,
    inconsistent_alive_identity: bool,
) -> &'static str {
    if inconsistent_alive_identity {
        "identity row is marked alive but its bound local session row is missing or dead"
    } else if !found {
        "no relay profile, local identity, or channel membership row matched this target"
    } else if !profile_found && (identity_found || found) {
        "pubkey is known locally, but no relay kind:0 profile is cached"
    } else {
        ""
    }
}
