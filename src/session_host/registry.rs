/// Returns `(slug, harness, byline)` for configured local agents.
pub fn spawnable_agents() -> Vec<(String, String, Option<String>)> {
    crate::identity::list_local_agents(&crate::config::mosaico_home())
        .into_iter()
        .map(|agent| (agent.slug, agent.harness, agent.byline))
        .collect()
}
