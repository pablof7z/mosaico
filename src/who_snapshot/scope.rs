/// Top-level work-root for `scope`.
pub(super) fn work_root_for(
    aggregation: &crate::who_aggregation::WhoAggregation,
    scope: &str,
) -> anyhow::Result<String> {
    aggregation.root_for_channel(scope)
}

pub(super) fn scope_contains_channel(
    aggregation: &crate::who_aggregation::WhoAggregation,
    current: &str,
    scope: &str,
) -> anyhow::Result<bool> {
    aggregation.scope_contains(current, scope)
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
