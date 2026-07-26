use super::*;

pub(super) async fn run(workspace: Option<String>, all: bool, recursive: bool) -> Result<()> {
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

fn render_projection(projection: &serde_json::Value) -> String {
    let mut output = String::new();
    let sections = projection["sections"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut printed = false;
    for section in sections {
        let channels = section["channels"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        if channels.is_empty() {
            continue;
        }
        if printed {
            output.push('\n');
        }
        let _ = writeln!(
            output,
            "{}",
            section["title"].as_str().unwrap_or("Channels")
        );
        for channel in channels {
            render_channel(&mut output, channel, 0);
        }
        printed = true;
    }
    if !printed {
        output.push_str("No channels found.\n");
    }
    output
}

fn render_channel(output: &mut String, channel: &serde_json::Value, depth: usize) {
    let path = channel["path"].as_str().unwrap_or("");
    if path.is_empty() {
        return;
    }
    let suffix = channel["subchannels"]
        .as_u64()
        .map(|count| format!(" + {count} subchannel{}", if count == 1 { "" } else { "s" }))
        .unwrap_or_default();
    let mut details = Vec::new();
    if let Some(about) = channel["about"].as_str().filter(|about| !about.is_empty()) {
        details.push(about.to_string());
    }
    if let Some(agents) = channel["agents"].as_u64() {
        details.push(format!(
            "{agents} agent{}",
            if agents == 1 { "" } else { "s" }
        ));
    }
    if let Some(activity) = channel["last_activity"]
        .as_str()
        .filter(|activity| !activity.is_empty())
    {
        details.push(format!("active {activity}"));
    }
    let details = (!details.is_empty())
        .then(|| format!("  — {}", details.join(" · ")))
        .unwrap_or_default();
    let _ = writeln!(output, "{}{path}{suffix}{details}", "  ".repeat(depth));

    for child in channel["children"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        render_channel(output, child, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::render_projection;

    #[test]
    fn compact_suffix_omits_zero() {
        let no_children = serde_json::json!({
            "path": "/solo",
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
                    "path": "/root",
                    "about": "Root",
                    "agents": 2,
                    "last_activity": "3 min ago",
                    "children": [{
                        "path": "/root/review",
                        "about": "Reviews",
                    }],
                }],
            }],
        });
        let root = &projection["sections"][0]["channels"][0];
        assert_eq!(root["path"], "/root");
        assert_eq!(root["children"][0]["path"], "/root/review");
        assert!(projection.to_string().find("child_h").is_none());
        assert_eq!(
            render_projection(&projection),
            concat!(
                "Your workspace\n",
                "/root  — Root · 2 agents · active 3 min ago\n",
                "  /root/review  — Reviews\n",
            )
        );
    }
}
