use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};

use crate::harness::{EnvDirective, Transport};

pub(super) enum CommandSource {
    Command(Vec<String>),
    Bundle(String),
}

pub(super) struct PtyLaunchRequest {
    pub(super) agent: String,
    pub(super) root: String,
    pub(super) channel: Option<String>,
    pub(super) session_name: Option<String>,
    pub(super) source: CommandSource,
    pub(super) extra_args: Vec<String>,
    pub(super) prompt: Option<String>,
}

struct PreparedCommand {
    id: Option<String>,
    argv: Vec<String>,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
}

pub(super) async fn launch(request: PtyLaunchRequest) -> Result<()> {
    let PtyLaunchRequest {
        agent,
        root,
        channel,
        session_name,
        source,
        extra_args,
        prompt,
    } = request;
    let prepared = prepare_command(&agent, source, extra_args)?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let preflight = super::super::daemon_call_async(
        "agent_launch_preflight",
        serde_json::json!({
            "agent": agent.clone(),
            "session_name": session_name,
        }),
    )
    .await
    .context("launch refused before spawning harness")?;
    let durable_reservation = preflight["durable_reservation"]
        .as_str()
        .map(str::to_string);
    let spawned = crate::pty::spawn_session(crate::pty::SpawnSessionArgs {
        id: prepared.id,
        agent: agent.clone(),
        root,
        cwd,
        channel,
        session_name,
        ephemeral: false,
        durable_reservation: durable_reservation.clone(),
        command: prepared.argv,
        env: prepared.env,
        env_remove: prepared.env_remove,
    });
    let meta = match spawned {
        Ok(meta) => meta,
        Err(error) => {
            if let Some(reservation) = durable_reservation {
                let _ = super::super::daemon_call_async(
                    "agent_launch_release",
                    serde_json::json!({ "durable_reservation": reservation }),
                )
                .await;
            }
            return Err(error);
        }
    };

    eprintln!("[tenex-edge pty] session: {}", meta.id);
    eprintln!("[tenex-edge pty] detach: close this attach terminal");
    eprintln!(
        "[tenex-edge pty] reattach: tenex-edge pty attach {}",
        meta.id
    );
    if let Some(prompt) = prompt {
        crate::session_host::inject_spawn_message(&meta.id, &prompt)
            .await
            .with_context(|| format!("injecting initial prompt into pty session {}", meta.id))?;
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("[tenex-edge pty] attach skipped: not running on a TTY");
        return Ok(());
    }
    crate::pty::attach(&meta.socket)
}

fn prepare_command(
    agent: &str,
    source: CommandSource,
    extra_args: Vec<String>,
) -> Result<PreparedCommand> {
    match source {
        CommandSource::Command(base) => {
            let extra =
                super::launch_command::extra_args_without_duplicate_suffix(&base, extra_args);
            Ok(PreparedCommand {
                id: None,
                argv: super::launch_command::append_launch_args(base, &extra),
                env: Vec::new(),
                env_remove: Vec::new(),
            })
        }
        CommandSource::Bundle(bundle) => prepare_bundle(agent, &bundle, extra_args),
    }
}

fn prepare_bundle(agent: &str, bundle: &str, extra_args: Vec<String>) -> Result<PreparedCommand> {
    let id = crate::pty::new_session_id(agent);
    let scratch = crate::config::edge_home()
        .join("harness-profiles")
        .join(&id);
    let resolved = crate::harness::resolve(bundle, &scratch)
        .with_context(|| format!("resolving PTY harness bundle {bundle:?}"))?;
    if resolved.transport != Transport::Pty {
        bail!(
            "harness bundle {bundle:?} uses the {} transport, which cannot attach to a terminal",
            resolved.transport.as_str()
        );
    }
    materialize_profile(&resolved.profile.files)?;

    let mut argv = resolved.base_argv;
    argv.extend(extra_args);
    let mut env = resolved.profile.extra_env;
    let mut env_remove = Vec::new();
    for directive in resolved.driver.base_env {
        match directive {
            EnvDirective::Set(key, value) => env.push((key.to_string(), value.to_string())),
            EnvDirective::Remove(key) => env_remove.push(key.to_string()),
        }
    }
    Ok(PreparedCommand {
        id: Some(id),
        argv,
        env,
        env_remove,
    })
}

fn materialize_profile(files: &[(PathBuf, String)]) -> Result<()> {
    for (path, contents) in files {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating profile dir {}", parent.display()))?;
        }
        std::fs::write(path, contents)
            .with_context(|| format!("writing profile file {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_source_deduplicates_forwarded_suffix() {
        let prepared = prepare_command(
            "codex",
            CommandSource::Command(vec!["codex".into(), "--yolo".into()]),
            vec!["--yolo".into()],
        )
        .unwrap();
        assert_eq!(prepared.argv, vec!["codex", "--yolo"]);
        assert!(prepared.env.is_empty());
    }
}
