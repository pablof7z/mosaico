use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub(super) struct ChannelNode {
    pub path: String,
    pub about: String,
    pub agents: Option<usize>,
    pub last_activity: Option<String>,
    pub children: Vec<ChannelNode>,
}

pub(super) async fn fetch_forest() -> Result<Vec<ChannelNode>> {
    let projection = crate::cli::daemon_call_async(
        "channel_list",
        crate::cli::rpc_params(serde_json::json!({
            "recursive": true,
        })),
    )
    .await
    .context("loading channels")?;
    Ok(parse_projection(&projection))
}

fn parse_projection(projection: &serde_json::Value) -> Vec<ChannelNode> {
    let mut roots = Vec::new();
    let sections = projection["sections"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for section in sections {
        for channel in section["channels"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if let Some(node) = parse_node(channel) {
                roots.push(node);
            }
        }
    }
    roots
}

fn parse_node(value: &serde_json::Value) -> Option<ChannelNode> {
    let path = value["path"]
        .as_str()
        .filter(|p| !p.is_empty())?
        .to_string();
    let about = value["about"].as_str().unwrap_or("").to_string();
    let agents = value["agents"].as_u64().map(|n| n as usize);
    let last_activity = value["last_activity"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let children = value["children"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(parse_node)
        .collect();
    Some(ChannelNode {
        path,
        about,
        agents,
        last_activity,
        children,
    })
}

pub(super) async fn edit_about(path: &str, about: &str) -> Result<()> {
    let v = crate::cli::daemon_call_async(
        "channel_edit",
        crate::cli::rpc_params(serde_json::json!({
            "channel": path,
            "about": about,
        })),
    )
    .await
    .with_context(|| format!("editing {path}"))?;
    if !v["confirmed"].as_bool().unwrap_or(false) {
        anyhow::bail!("relay did not confirm about update for {path}");
    }
    Ok(())
}

pub(super) async fn delete_channel(path: &str) -> Result<serde_json::Value> {
    crate::cli::daemon_call_async(
        "channel_delete",
        crate::cli::rpc_params(serde_json::json!({ "channel": path })),
    )
    .await
    .with_context(|| format!("deleting {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_public_paths() {
        let projection = serde_json::json!({
            "sections": [{
                "channels": [{
                    "path": "#root",
                    "about": "Root",
                    "agents": 2,
                    "last_activity": "3 min ago",
                    "children": [{
                        "path": "#root/review",
                        "about": "Reviews",
                        "children": [],
                    }],
                }],
            }],
        });
        let roots = parse_projection(&projection);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].path, "#root");
        assert_eq!(roots[0].agents, Some(2));
        assert_eq!(roots[0].children[0].path, "#root/review");
        assert_eq!(roots[0].children[0].about, "Reviews");
    }
}
