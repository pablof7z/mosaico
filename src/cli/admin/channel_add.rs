//! `channel add` — add a member to a channel in one of two shapes:
//!   * human by id:        `channel add <pubkey|npub|nip05> <channel> [--admin]`
//!   * pull an existing one:`channel add --session <npub|hex|current-handle> <channel>`
//!
//! Human adds route to the daemon's `channel_add_member`; existing-session adds
//! route to `invite`. `--message` posts a chat mentioning the brought-online
//! session and is valid only with `--session`.

use super::args::AddArgs;
use super::*;

pub(super) async fn channel_add(a: AddArgs) -> Result<()> {
    match a.session.clone() {
        Some(handle) => session_add(a.first, &handle, a.message).await,
        None => human_add(a.first, a.second, a.admin, a.message).await,
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
        anyhow::bail!("--message is only valid with --session");
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

async fn session_add(channel: Option<String>, handle: &str, message: Option<String>) -> Result<()> {
    let Some(channel) = channel else {
        anyhow::bail!("channel add --session <npub|hex|current-handle> <channel>");
    };
    // Strip the mention sigil before sending the canonical handle to the daemon.
    let selector = handle.strip_prefix('@').unwrap_or(handle);
    let v = invite_call(
        &channel,
        serde_json::json!({ "session": selector }),
        message,
    )
    .await?;
    if v["ambiguous"].is_array() {
        print_ambiguous(&format!("--session {handle}"), &channel, &v);
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

/// The brought-online session's canonical public handle from an invite response.
fn online_label(v: &serde_json::Value) -> String {
    v["online_agent"].as_str().unwrap_or("session").to_string()
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

    #[test]
    fn new_session_flag_stays_removed() {
        let kind = parse_err("tenex-edge channel add --new-session reviewer ops");
        assert_eq!(kind, ErrorKind::UnknownArgument);
    }

    #[test]
    fn session_pull_accepts_handle_and_message() {
        let a = parse_add(
            "tenex-edge channel add --session @sable-grove-179-coder ops --message welcome",
        );
        assert_eq!(a.session.as_deref(), Some("@sable-grove-179-coder"));
        assert_eq!(a.first.as_deref(), Some("ops"));
        assert_eq!(a.message.as_deref(), Some("welcome"));
    }

    #[test]
    fn human_takes_two_positionals_and_admin() {
        let a = parse_add("tenex-edge channel add npub1xyz ops --admin");
        assert_eq!(a.first.as_deref(), Some("npub1xyz"));
        assert_eq!(a.second.as_deref(), Some("ops"));
        assert!(a.admin && a.session.is_none());
    }

    #[test]
    fn admin_conflicts_with_session_mode() {
        let kind = parse_err("tenex-edge channel add --session @sable-grove-179-coder ops --admin");
        assert_eq!(kind, ErrorKind::ArgumentConflict);
    }

    // `--message` is only meaningful in session mode (it mentions the
    // brought-online session). In human mode it has no target, so dispatch must
    // reject it BEFORE any daemon round-trip. Guarded here because the check is a
    // runtime dispatch guard, not a clap-level conflict.
    #[tokio::test]
    async fn message_rejected_in_human_mode() {
        let err = super::human_add(
            Some("npub1example".into()),
            Some("ops".into()),
            false,
            Some("hello".into()),
        )
        .await
        .expect_err("--message with a human target must be rejected");
        assert!(
            err.to_string()
                .contains("--message is only valid with --session"),
            "unexpected error: {err}"
        );
    }
}
