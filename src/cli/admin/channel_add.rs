//! `channel add` — add a member to a channel in one of three shapes:
//!   * human by id:        `channel add <pubkey|npub|nip05> <channel> [--admin]`
//!   * spawn a new session: `channel add --new-session <role>[@machine] <channel>`
//!   * pull an existing one:`channel add --session @codename@host <channel>`
//!
//! Human adds route to the daemon's `channel_add_member`; both session modes route to
//! `invite` (fresh spawn vs. resume/pull). `--message` posts a chat mentioning
//! the brought-online session and is valid only in the session modes.

use super::args::AddArgs;
use super::*;

pub(super) async fn channel_add(a: AddArgs) -> Result<()> {
    match (a.new_session.clone(), a.session.clone()) {
        (Some(role), None) => new_session_add(a.first, &role, a.message).await,
        (None, Some(codehost)) => session_add(a.first, &codehost, a.message).await,
        (None, None) => human_add(a.first, a.second, a.admin, a.message).await,
        // clap `conflicts_with_all` makes both flags together unreachable.
        (Some(_), Some(_)) => anyhow::bail!("--new-session and --session are mutually exclusive"),
    }
}

fn print_ambiguous(re_run_target: &str, channel: &str, v: &serde_json::Value) -> ! {
    let name = v["reference"].as_str().unwrap_or(channel);
    eprintln!("'{name}' is ambiguous — re-run with an exact channel:");
    if let Some(refs) = v["ambiguous"].as_array() {
        for r in refs.iter().filter_map(|r| r.as_str()) {
            eprintln!("  tenex-edge channel add {re_run_target} {r}");
        }
    }
    std::process::exit(2);
}

async fn human_add(
    id: Option<String>,
    channel: Option<String>,
    admin: bool,
    message: Option<String>,
) -> Result<()> {
    if message.is_some() {
        anyhow::bail!("--message is only valid with --new-session or --session");
    }
    let (Some(id), Some(channel)) = (id, channel) else {
        anyhow::bail!("channel add <pubkey|npub|nip05> <channel> [--admin]");
    };
    let v = daemon_call_async(
        "channel_add_member",
        crate::cli::rpc_params(serde_json::json!({
            "channel": channel,
            "pubkey": id,
            "admin": admin,
        })),
    )
    .await?;
    if v["ambiguous"].is_array() {
        print_ambiguous(&id, &channel, &v);
    }
    let role = v["role"]
        .as_str()
        .unwrap_or(if admin { "admin" } else { "member" });
    println!("added {} to #{channel} as {role}", id.bold());
    Ok(())
}

async fn new_session_add(
    channel: Option<String>,
    role: &str,
    message: Option<String>,
) -> Result<()> {
    let Some(channel) = channel else {
        anyhow::bail!("channel add --new-session <role>[@machine] <channel>");
    };
    let v = invite_call(
        &channel,
        serde_json::json!({ "target_agent": role }),
        message,
    )
    .await?;
    if v["ambiguous"].is_array() {
        print_ambiguous(&format!("--new-session {role}"), &channel, &v);
    }
    // Synchronous success line: the fresh session is confirmed online, named by
    // its own codename.
    println!(
        "a {role} is now on #{channel}: @{}",
        online_label(&v).bold()
    );
    warn_message_error(&v);
    Ok(())
}

async fn session_add(
    channel: Option<String>,
    codehost: &str,
    message: Option<String>,
) -> Result<()> {
    let Some(channel) = channel else {
        anyhow::bail!("channel add --session @codename@host <channel>");
    };
    // Strip a leading `@` so both `@codename@host` and `codename@host` are accepted.
    let selector = codehost.strip_prefix('@').unwrap_or(codehost);
    let v = invite_call(
        &channel,
        serde_json::json!({ "session": selector }),
        message,
    )
    .await?;
    if v["ambiguous"].is_array() {
        print_ambiguous(&format!("--session {codehost}"), &channel, &v);
    }
    println!("@{} is now on #{channel}", online_label(&v).bold());
    warn_message_error(&v);
    Ok(())
}

/// Call the daemon `invite` RPC with the channel + a target selector, threading
/// `--message` through as `add_message`.
async fn invite_call(
    channel: &str,
    target: serde_json::Value,
    message: Option<String>,
) -> Result<serde_json::Value> {
    let mut params = serde_json::json!({ "channel": channel, "add_message": message });
    if let (Some(obj), Some(t)) = (params.as_object_mut(), target.as_object()) {
        for (k, val) in t {
            obj.insert(k.clone(), val.clone());
        }
    }
    daemon_call_async("invite", crate::cli::rpc_params(params)).await
}

/// The brought-online session's `codename@host` handle from an invite response.
/// Remote responses already carry `codename@backend`; local ones need the host
/// appended.
fn online_label(v: &serde_json::Value) -> String {
    let label = v["online_agent"].as_str().unwrap_or("session");
    let host = v["host"].as_str().unwrap_or("");
    if label.contains('@') || host.is_empty() {
        label.to_string()
    } else {
        format!("{label}@{host}")
    }
}

fn warn_message_error(v: &serde_json::Value) {
    if let Some(err) = v["message_error"].as_str().filter(|s| !s.is_empty()) {
        eprintln!(
            "{} the member was added, but posting the message failed: {err}",
            "warning:".yellow()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::AddArgs;
    use crate::cli::admin::ChannelAction;
    use crate::cli::args::{Cli, Cmd};
    use clap::{error::ErrorKind, Parser};

    fn split(line: &str) -> Vec<&str> {
        line.split_whitespace().collect()
    }

    fn parse_add(line: &str) -> AddArgs {
        match Cli::try_parse_from(split(line))
            .expect("channel add parses")
            .cmd
        {
            Cmd::Channel {
                action: ChannelAction::Add(a),
            } => a,
            _ => panic!("expected channel add command"),
        }
    }

    fn parse_err(line: &str) -> ErrorKind {
        match Cli::try_parse_from(split(line)) {
            Ok(_) => panic!("expected parse failure for {line:?}"),
            Err(err) => err.kind(),
        }
    }

    // Flag modes take ONE positional (the channel); the human mode takes TWO.
    #[test]
    fn new_session_takes_one_positional_channel() {
        let a = parse_add("tenex-edge channel add --new-session reviewer ops");
        assert_eq!(a.new_session.as_deref(), Some("reviewer"));
        assert_eq!(a.first.as_deref(), Some("ops"));
        assert!(a.second.is_none() && a.session.is_none() && !a.admin);
    }

    #[test]
    fn session_pull_accepts_codename_and_message() {
        let a = parse_add(
            "tenex-edge channel add --session @bright-otter-042@laptop ops --message welcome",
        );
        assert_eq!(a.session.as_deref(), Some("@bright-otter-042@laptop"));
        assert_eq!(a.first.as_deref(), Some("ops"));
        assert_eq!(a.message.as_deref(), Some("welcome"));
    }

    #[test]
    fn human_takes_two_positionals_and_admin() {
        let a = parse_add("tenex-edge channel add npub1xyz ops --admin");
        assert_eq!(a.first.as_deref(), Some("npub1xyz"));
        assert_eq!(a.second.as_deref(), Some("ops"));
        assert!(a.admin && a.new_session.is_none() && a.session.is_none());
    }

    #[test]
    fn admin_conflicts_with_flag_modes() {
        let kind = parse_err("tenex-edge channel add --new-session reviewer ops --admin");
        assert_eq!(kind, ErrorKind::ArgumentConflict);
    }

    #[test]
    fn new_session_conflicts_with_session() {
        let kind =
            parse_err("tenex-edge channel add --new-session reviewer --session @code@host ops");
        assert_eq!(kind, ErrorKind::ArgumentConflict);
    }
}
