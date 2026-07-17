mod build;
mod model;
mod render;

pub(crate) use build::AgentWhoInput;

pub(crate) fn render_agent_who(
    store: &crate::state::Store,
    input: AgentWhoInput<'_>,
) -> anyhow::Result<String> {
    let aggregation = crate::who_aggregation::WhoAggregation::load(store, input.now)?;
    render_agent_who_from_aggregation(&aggregation, input)
}

pub(crate) fn render_agent_who_from_aggregation(
    aggregation: &crate::who_aggregation::WhoAggregation,
    input: AgentWhoInput<'_>,
) -> anyhow::Result<String> {
    Ok(render::render_agent_who(&build::build_agent_who(
        aggregation,
        input,
    )?))
}

#[cfg(test)]
mod tests;
