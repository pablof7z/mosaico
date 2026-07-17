/// Returns `(slug, bundle, byline)` for configured local agents.
pub fn spawnable_agents() -> Vec<(String, String, Option<String>)> {
    crate::identity::list_local_agents(&crate::config::mosaico_home())
        .into_iter()
        .map(|(slug, harness, _profile, byline)| (slug, harness, byline))
        .collect()
}
