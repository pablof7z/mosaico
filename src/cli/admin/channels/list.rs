use super::*;
use std::io::IsTerminal;

pub(super) async fn run(workspace: Option<String>, all: bool, recursive: bool) -> Result<()> {
    let flagged = workspace.is_some() || all || recursive;
    // Operator interactive home: bare `channel list` opens the manager TUI.
    // Agent sessions and any explicit list flag keep the text projection.
    if !flagged && !in_agent_session() && stdin_stdout_tty() {
        return crate::cli::interactive::channel_picker::run().await;
    }
    if !flagged && !in_agent_session() && !stdin_stdout_tty() {
        // Non-interactive operator calls need an explicit inventory shape.
        anyhow::bail!(
            "channel list outside a mosaico session needs a list flag \
             (-a, -r, or --workspace) when not run in a terminal"
        );
    }

    let projection = daemon_call_async(
        "channel_list",
        crate::cli::rpc_params(serde_json::json!({
            "workspace": workspace,
            "all": all,
            "recursive": recursive,
        })),
    )
    .await?;
    print!("{}", render_projection(&projection));
    Ok(())
}

fn in_agent_session() -> bool {
    crate::cli::agent_env_slug().is_some()
        || std::env::var("MOSAICO_PUBKEY")
            .ok()
            .is_some_and(|value| !value.is_empty())
}

fn stdin_stdout_tty() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn render_projection(projection: &serde_json::Value) -> String {
    crate::channel_render::format_channel_list(projection)
}

#[cfg(test)]
mod tests {
    use super::render_projection;

    #[test]
    fn compact_suffix_omits_zero() {
        let no_children = serde_json::json!({
            "path": "#solo",
            "about": "Solo",
            "children": [],
        });
        assert!(no_children["subchannels"].is_null());
    }

    #[test]
    fn projection_shape_uses_public_paths_and_nested_children() {
        let projection = serde_json::json!({
            "sections": [{
                "kind": "own",
                "title": "Your workspace",
                "channels": [{
                    "path": "#root",
                    "about": "Root",
                    "agents": 2,
                    "last_activity": "3 min ago",
                    "children": [{
                        "path": "#root/review",
                        "about": "Reviews",
                    }],
                }],
            }],
        });
        let root = &projection["sections"][0]["channels"][0];
        assert_eq!(root["path"], "#root");
        assert_eq!(root["children"][0]["path"], "#root/review");
        assert!(projection.to_string().find("child_h").is_none());
        assert_eq!(
            render_projection(&projection),
            concat!(
                "Your workspace\n",
                "#root  — Root · 2 agents · active 3 min ago\n",
                "  #root/review  — Reviews\n",
            )
        );
    }
}
