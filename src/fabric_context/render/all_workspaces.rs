use super::render_workspace;
use crate::fabric_context::model::FabricView;

#[allow(dead_code)]
pub(in crate::fabric_context) fn render_views(views: &[FabricView]) -> String {
    let mut out = String::from("<mosaico>");

    for view in views {
        render_workspace(&mut out, view);
    }

    out.push_str("\n</mosaico>");
    out
}
