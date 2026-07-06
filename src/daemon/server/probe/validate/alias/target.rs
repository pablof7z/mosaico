use super::AliasTarget;

pub(in crate::daemon::server::probe::validate) fn alias_target(
    target: &str,
) -> Option<AliasTarget> {
    if let Some(rest) = target.strip_prefix("alias:") {
        let parts = rest.splitn(3, ':').collect::<Vec<_>>();
        return alias_parts(parts.first()?, parts.get(1)?, parts.get(2)?);
    }
    if let Some(rest) = target.strip_prefix("alias/") {
        let parts = rest.splitn(3, '/').collect::<Vec<_>>();
        return alias_parts(parts.first()?, parts.get(1)?, parts.get(2)?);
    }
    harnessed(target, "harness_session:", "harness_session")
        .or_else(|| harnessed(target, "harness-session:", "harness_session"))
        .or_else(|| harnessed_path(target, "harness_session/", "harness_session"))
        .or_else(|| harnessed_path(target, "harness-session/", "harness_session"))
        .or_else(|| harnessed(target, "resume:", "resume"))
        .or_else(|| harnessed_path(target, "resume/", "resume"))
        .or_else(|| machine_wide(target, "pty_session:", "pty_session"))
        .or_else(|| machine_wide(target, "pty-session:", "pty_session"))
        .or_else(|| machine_wide(target, "pty:", "pty_session"))
        .or_else(|| machine_wide(target, "pty_session/", "pty_session"))
        .or_else(|| machine_wide(target, "pty-session/", "pty_session"))
        .or_else(|| machine_wide(target, "pty/", "pty_session"))
        .or_else(|| machine_wide(target, "watch_pid:", "watch_pid"))
        .or_else(|| machine_wide(target, "watch-pid:", "watch_pid"))
        .or_else(|| machine_wide(target, "watch_pid/", "watch_pid"))
        .or_else(|| machine_wide(target, "watch-pid/", "watch_pid"))
}

fn alias_parts(harness: &str, kind: &str, external_id: &str) -> Option<AliasTarget> {
    (!harness.trim().is_empty() && !kind.trim().is_empty() && !external_id.trim().is_empty()).then(
        || AliasTarget {
            harness: Some(harness.to_string()),
            kind: normalize_kind(kind).to_string(),
            external_id: external_id.to_string(),
        },
    )
}

fn harnessed(target: &str, prefix: &str, kind: &str) -> Option<AliasTarget> {
    let rest = target.strip_prefix(prefix)?;
    let (harness, external_id) = rest.split_once(':')?;
    alias_parts(harness, kind, external_id)
}

fn harnessed_path(target: &str, prefix: &str, kind: &str) -> Option<AliasTarget> {
    let rest = target.strip_prefix(prefix)?;
    let (harness, external_id) = rest.split_once('/')?;
    alias_parts(harness, kind, external_id)
}

fn machine_wide(target: &str, prefix: &str, kind: &str) -> Option<AliasTarget> {
    let external_id = target.strip_prefix(prefix)?;
    (!external_id.trim().is_empty()).then(|| AliasTarget {
        harness: None,
        kind: kind.to_string(),
        external_id: external_id.to_string(),
    })
}

fn normalize_kind(kind: &str) -> &str {
    match kind {
        "harness-session" => "harness_session",
        "pty-session" | "pty" => "pty_session",
        "watch-pid" => "watch_pid",
        other => other,
    }
}
