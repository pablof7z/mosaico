/// Top-level work-root for `scope`.
pub(super) fn work_root_for(
    aggregation: &crate::who_aggregation::WhoAggregation,
    scope: &str,
) -> anyhow::Result<String> {
    aggregation.root_for_channel(scope)
}

/// Whether `scope` belongs to the workspace rooted at `current`. `None` means
/// not yet known: the view has not delivered the channel's whole ancestry
/// (as while a restarted daemon rehydrates). That channel is left out of this
/// snapshot rather than failing it; it appears once its ancestry resolves.
pub(super) fn scope_contains_channel(
    aggregation: &crate::who_aggregation::WhoAggregation,
    current: &str,
    scope: &str,
) -> Option<bool> {
    match aggregation.scope_contains(current, scope) {
        Ok(contains) => Some(contains),
        Err(error) => {
            tracing::debug!(%scope, %error, "who: channel ancestry not yet known");
            None
        }
    }
}

pub(super) fn is_archived_channel(
    aggregation: &crate::who_aggregation::WhoAggregation,
    scope: &str,
) -> bool {
    aggregation.is_archived(scope)
}

pub(super) fn is_root_channel(
    aggregation: &crate::who_aggregation::WhoAggregation,
    scope: &str,
) -> bool {
    aggregation.is_root(scope)
}
