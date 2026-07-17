pub(in crate::cli) struct LaunchRequest {
    pub(in crate::cli) agent: String,
    pub(in crate::cli) root: Option<String>,
    pub(in crate::cli) channel: Option<String>,
    pub(in crate::cli) session_name: Option<String>,
    pub(in crate::cli) prompt: Option<String>,
}
