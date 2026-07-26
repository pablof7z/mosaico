use super::*;

pub(in crate::fabric_context) fn presence_rows(
    inputs: &ViewInputs,
    channel: &str,
    cursor: u64,
    now: u64,
) -> Vec<PresenceRow> {
    if cursor == 0 {
        return Vec::new();
    }
    inputs
        .presence
        .statuses
        .get(channel)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|status| {
            inputs
                .members
                .roster
                .get(channel)
                .is_some_and(|roster| roster.contains_key(&status.pubkey))
        })
        .filter_map(|status| presence_row(inputs, status, cursor, now))
        .collect()
}

fn presence_row(
    inputs: &ViewInputs,
    status: &StatusCap,
    cursor: u64,
    now: u64,
) -> Option<PresenceRow> {
    let presence = projected_presence(status, now);
    let changed_at = if presence.state == status.state {
        status.changed_at
    } else {
        status.changed_at.max(presence.state_since)
    };
    let native_failure = status
        .native_failure
        .as_ref()
        .filter(|failure| failure.finished_at > cursor && failure.finished_at <= now)
        .map(|failure| NativeFailureRow {
            outcome: failure.outcome.clone(),
            message: failure.message.clone(),
            since: relative_time(failure.finished_at, now),
        });
    if (changed_at <= cursor || changed_at > now) && native_failure.is_none() {
        return None;
    }
    presence_snapshot_row_with_failure(inputs, status, now, native_failure)
}

pub(in crate::fabric_context) fn presence_snapshot_row(
    inputs: &ViewInputs,
    status: &StatusCap,
    now: u64,
) -> Option<PresenceRow> {
    if status.changed_at > now {
        return None;
    }
    let native_failure = status
        .native_failure
        .as_ref()
        .filter(|failure| failure.finished_at <= now)
        .map(|failure| NativeFailureRow {
            outcome: failure.outcome.clone(),
            message: failure.message.clone(),
            since: relative_time(failure.finished_at, now),
        });
    presence_snapshot_row_with_failure(inputs, status, now, native_failure)
}

fn presence_snapshot_row_with_failure(
    inputs: &ViewInputs,
    status: &StatusCap,
    now: u64,
    native_failure: Option<NativeFailureRow>,
) -> Option<PresenceRow> {
    if status.pubkey == inputs.meta.self_pubkey {
        return None;
    }
    let presence = projected_presence(status, now);
    let text = presence.text();
    if text.is_empty() && native_failure.is_none() {
        return None;
    }
    let (name, host, workspace) = member_origin(
        presence_reference(inputs, status),
        &status.host,
        &status.workspace,
        inputs,
    );
    Some(PresenceRow {
        name,
        host,
        workspace,
        branch: status.branch.clone(),
        state: presence.state,
        status: text,
        since: relative_time(presence.state_since, now),
        native_failure,
    })
}

fn member_origin(
    mut name: String,
    host: &str,
    workspace: &str,
    inputs: &ViewInputs,
) -> (String, String, String) {
    let self_workspace = inputs
        .meta
        .self_row
        .as_ref()
        .map(|row| row.workspace.as_str())
        .unwrap_or(inputs.meta.current_workspace.trim());
    let cross_workspace = !workspace.is_empty() && workspace != self_workspace;
    let cross_host = !host.is_empty() && host != inputs.meta.local_host.trim();
    if !cross_workspace && !cross_host {
        return (name, String::new(), String::new());
    }
    if !host.is_empty() {
        let suffix = format!("@{host}");
        if let Some(bare) = name.strip_suffix(&suffix) {
            name = bare.to_string();
        }
    }
    (
        name,
        host.to_string(),
        if cross_workspace {
            workspace.to_string()
        } else {
            String::new()
        },
    )
}

fn presence_reference(inputs: &ViewInputs, status: &StatusCap) -> String {
    if !status.slug.trim().is_empty() {
        return status.slug.clone();
    }
    inputs
        .members
        .refs
        .get(&status.pubkey)
        .cloned()
        .unwrap_or_default()
}

pub(in crate::fabric_context) fn projected_presence(
    status: &StatusCap,
    now: u64,
) -> crate::session_presence::PublicPresence {
    crate::session_presence::observed(
        status.state,
        status.state_since,
        &status.title,
        &status.activity,
        status.observed_at,
        status.expiration,
        now,
    )
}
