use super::{render_agents, render_workspace};
use crate::fabric_context::model::{shared_agents, workspace_agents, FabricView};

#[allow(dead_code)]
pub(in crate::fabric_context) fn render_views(views: &[FabricView]) -> String {
    let shared = shared_agents(views);
    let mut out = String::from("<tenex-edge>");
    render_agents(&mut out, &shared, "available-agents");

    for view in views {
        let additions = workspace_agents(view, &shared);
        render_workspace(&mut out, view, &additions, "workspace-agents");
    }

    out.push_str("\n</tenex-edge>");
    out
}
