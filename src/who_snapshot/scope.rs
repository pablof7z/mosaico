use super::StoreReader;

/// Top-level work-root for `scope`.
pub(super) fn work_root_for(store: StoreReader<'_>, scope: &str) -> String {
    store
        .channel_project_root(scope)
        .unwrap_or_else(|e| {
            tracing::error!(
                channel = %scope,
                error = ?e,
                "who snapshot: channel ancestry lookup failed walking work-root"
            );
            None
        })
        .unwrap_or_else(|| scope.to_string())
}

pub(super) fn scope_contains_channel(store: StoreReader<'_>, current: &str, scope: &str) -> bool {
    if is_archived_channel(store, current) || is_archived_channel(store, scope) {
        return false;
    }
    if current == scope {
        return true;
    }
    matches!(store.is_root_channel(current), Ok(true)) && work_root_for(store, scope) == current
}

pub(super) fn is_archived_channel(store: StoreReader<'_>, scope: &str) -> bool {
    match store.get_channel(scope) {
        Ok(Some(channel)) => channel.is_archived(),
        Ok(None) => false,
        Err(e) => {
            tracing::error!(
                channel = %scope,
                error = ?e,
                "who snapshot: archived-channel lookup failed; treating channel as active"
            );
            false
        }
    }
}

pub(super) fn is_root_channel(store: StoreReader<'_>, scope: &str) -> bool {
    match store.is_root_channel(scope) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(
                channel = %scope,
                error = ?e,
                "who snapshot: is_root_channel lookup failed; assuming non-root"
            );
            false
        }
    }
}
