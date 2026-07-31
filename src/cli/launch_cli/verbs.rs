use super::args::LaunchRequest;
use super::selection::resolve_fresh_agent;
use anyhow::{Context as _, Result};

// ── launch ───────────────────────────────────────────────────────────────────

/// Attach or resume a named session, or launch a fresh harness if it is unknown.
///
/// A fresh launch delegates agent discovery and transport selection to the
/// daemon, then attaches PTY sessions or reports headless RPC sessions.
pub(in crate::cli) async fn launch(request: LaunchRequest) -> Result<()> {
    if super::existing::launch_if_known(&request).await? {
        return Ok(());
    }
    let LaunchRequest {
        agent: requested_agent,
        channel,
        session_name,
        prompt,
        extra_args,
    } = request;
    let cwd = std::env::current_dir().unwrap_or_default();
    let selection = resolve_fresh_agent(&requested_agent, &cwd).await?;
    let agent = selection.slug;
    let root = crate::daemon::workspace_path::channel_for_path_optional(&cwd)?;
    let channel = resolve_launch_channel(root.as_deref(), &agent, channel).await?;
    super::fresh::launch(super::fresh::FreshLaunchRequest {
        agent,
        root: root.unwrap_or_default(),
        channel,
        session_name,
        prompt,
        extra_args,
    })
    .await
}

/// Resolve the launch channel shared by the PTY and ACP paths. `--channel ""`
/// opens the interactive picker (TTY required); a bare launch defaults to the
/// workspace channel; a name is resolved to its opaque `channel_h` (created if
/// absent) before spawning, so admission and provisioning use the same route.
async fn resolve_launch_channel(
    root: Option<&str>,
    agent: &str,
    channel: Option<String>,
) -> Result<Option<String>> {
    let Some(root) = root else {
        if channel.is_some() {
            anyhow::bail!(
                "an unscoped launch cannot select a channel; launch from a known workspace \
                 or run `mosaico channel init` first"
            );
        }
        return Ok(None);
    };
    let want_picker = matches!(channel, Some(ref s) if s.is_empty());
    if want_picker {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "channel selection needs a TTY to open the interactive picker; \
                 pass --channel <name> to scope into a specific channel non-interactively"
            );
        }
        return Ok(Some(pick_channel(root, agent).await?));
    }
    match channel {
        None => Ok(Some(root.to_string())),
        Some(name) if !name.is_empty() => {
            let v = super::super::daemon_call_async(
                "channel_resolve",
                channel_resolve_params(root, &name, agent),
            )
            .await?;
            Ok(Some(
                v["channel_h"]
                    .as_str()
                    .context("channel_resolve did not return channel_h")?
                    .to_string(),
            ))
        }
        other => Ok(other),
    }
}

fn channel_resolve_params(root: &str, name: &str, agent: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": root,
        "name": name,
        "agent": agent,
        "create_if_absent": true,
    })
}

/// Fetch all public channel paths under `root` and present a fuzzy picker.
/// Here `root` is the top-level channel backing the user-facing workspace.
/// Includes a "＋ Create new channel…" entry at the top; selecting it prompts
/// for a name and creates the channel through the daemon. The selected public
/// path resolves to an internal id only at the private launch boundary.
async fn pick_channel(root: &str, agent_slug: &str) -> Result<String> {
    let v = super::super::daemon_call_async(
        "channel_list",
        crate::cli::rpc_params(serde_json::json!({
            "workspace": root,
            "all": false,
            "recursive": false,
        })),
    )
    .await?;

    // "＋ Create…" is always the first item so it's reachable by typing its name.
    const CREATE: &str = "＋  Create new channel…";
    let mut paths: Vec<Option<String>> = vec![None]; // None = create sentinel
    let mut labels: Vec<String> = vec![CREATE.to_string()];
    for workspace in v["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["channels"].as_array().into_iter().flatten())
    {
        collect_picker_channels(&workspace["children"], 0, &mut paths, &mut labels);
    }

    let theme = dialoguer::theme::ColorfulTheme::default();
    let idx = dialoguer::FuzzySelect::with_theme(&theme)
        .with_prompt("Select channel")
        .items(&labels)
        .default(0)
        .interact()?;

    match &paths[idx] {
        Some(path) => resolve_existing_channel_path(root, path, agent_slug).await,
        None => create_channel_interactive(root, agent_slug, &theme).await,
    }
}

fn collect_picker_channels(
    channels: &serde_json::Value,
    depth: usize,
    paths: &mut Vec<Option<String>>,
    labels: &mut Vec<String>,
) {
    for channel in channels.as_array().into_iter().flatten() {
        let Some(path) = channel["path"].as_str().filter(|path| !path.is_empty()) else {
            continue;
        };
        labels.push(format!("{}{}", "  ".repeat(depth), path));
        paths.push(Some(path.to_string()));
        collect_picker_channels(&channel["children"], depth + 1, paths, labels);
    }
}

async fn resolve_existing_channel_path(root: &str, path: &str, agent: &str) -> Result<String> {
    let prefix = crate::channel_ref::format_channel_ref(root, &[]);
    let remainder = path
        .strip_prefix(&prefix)
        .with_context(|| format!("selected channel path {path:?} is outside #{root}"))?;
    anyhow::ensure!(
        remainder.is_empty() || remainder.starts_with('/'),
        "selected channel path {path:?} is outside #{root}"
    );
    let mut parent = root.to_string();
    for name in remainder.split('/').filter(|segment| !segment.is_empty()) {
        let v = super::super::daemon_call_async(
            "channel_resolve",
            serde_json::json!({
                "channel": parent,
                "name": name,
                "agent": agent,
                "create_if_absent": false,
            }),
        )
        .await?;
        parent = v["channel_h"]
            .as_str()
            .context("channel_resolve did not return channel_h")?
            .to_string();
    }
    Ok(parent)
}

/// Prompt for a channel name, then create it via the daemon using the agent
/// being launched and the local backend pubkey. Resolves the returned public
/// path through the private launch boundary before returning the internal id.
async fn create_channel_interactive(
    root: &str,
    agent_slug: &str,
    theme: &dialoguer::theme::ColorfulTheme,
) -> Result<String> {
    let name: String = dialoguer::Input::with_theme(theme)
        .with_prompt("Channel name")
        .interact_text()?;

    // Resolve the local backend config label from the daemon so the picker uses
    // the same backend identifier as `mosaico channel create --agent`.
    let backend_v = super::super::daemon_call_async("local_backend", serde_json::json!({})).await?;
    let backend_label = backend_v["backend_label"]
        .as_str()
        .context("local_backend did not return backend_label")?;

    let v = super::super::daemon_call_async(
        "channel_create",
        crate::cli::rpc_params(serde_json::json!({
            "channel": crate::channel_ref::format_channel_ref(root, std::slice::from_ref(&name)),
            "about": &name,
            "agents": [{ "slug": agent_slug, "backend": backend_label }],
        })),
    )
    .await?;

    let channel = v["channel"]
        .as_str()
        .context("channel_create did not return channel")?
        .to_string();
    eprintln!("created {channel}");
    resolve_existing_channel_path(root, &channel, agent_slug).await
}

#[cfg(test)]
mod tests {
    use super::{channel_resolve_params, collect_picker_channels, resolve_launch_channel};

    #[test]
    fn named_launch_channel_uses_channel_resolve_contract() {
        assert_eq!(
            channel_resolve_params("nmp", "forensic", "codex"),
            serde_json::json!({
                "channel": "nmp",
                "name": "forensic",
                "agent": "codex",
                "create_if_absent": true,
            })
        );
    }

    #[test]
    fn picker_collects_only_public_nested_paths() {
        let children = serde_json::json!([
            {
                "path": "#nmp/review",
                "about": "Reviews",
                "children": [{
                    "path": "#nmp/review/deep",
                    "about": "Deep reviews",
                }],
            }
        ]);
        let mut paths = vec![None];
        let mut labels = vec!["create".to_string()];

        collect_picker_channels(&children, 0, &mut paths, &mut labels);

        assert_eq!(
            paths,
            vec![
                None,
                Some("#nmp/review".into()),
                Some("#nmp/review/deep".into())
            ]
        );
        assert_eq!(labels, ["create", "#nmp/review", "  #nmp/review/deep"]);
    }

    #[tokio::test]
    async fn unknown_workspace_launches_unscoped_without_a_channel() {
        assert_eq!(
            resolve_launch_channel(None, "codex", None).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn unknown_workspace_cannot_resolve_a_relative_channel() {
        let error = resolve_launch_channel(None, "codex", Some("ops".into()))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unscoped launch"), "{error:#}");
    }
}
