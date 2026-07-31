//! Parser for backend-addressed management commands.

use super::ManagementCommand;
use anyhow::{Context, Result};

pub(super) fn parse_command(body: &str) -> Result<ManagementCommand> {
    let body = strip_leading_inline_mentions(body.trim());
    // Quote-aware split that does NOT treat `#` as a comment — channel paths
    // are `#root/child` and appear unquoted in chat management commands.
    let words = split_words(body).context("could not parse command quoting")?;
    match words.as_slice() {
        [verb, spec] if eq(verb, "add") => {
            crate::idref::parse_agent_backend_ref(spec)
                .with_context(|| format!("malformed agent spec {spec:?}"))?;
            Ok(ManagementCommand::Add { spec: spec.clone() })
        }
        [a, b] if eq(a, "list") && eq(b, "agents") => Ok(ManagementCommand::ListAgents),
        [a, b] if eq(a, "list") && eq(b, "sessions") => {
            Ok(ManagementCommand::ListSessions {
                all_channels: false,
            })
        }
        [a, b, c] if eq(a, "list") && eq(b, "all") && eq(c, "sessions") => {
            Ok(ManagementCommand::ListSessions { all_channels: true })
        }
        [verb, id] if eq(verb, "kill") => {
            let selector = id.trim();
            if selector.is_empty() {
                anyhow::bail!("kill requires a session npub, hex pubkey, or current handle");
            }
            if selector.starts_with("mosaico-") || selector.starts_with('$') {
                anyhow::bail!("raw session ids are internal; use the npub, hex pubkey, or handle");
            }
            Ok(ManagementCommand::Kill { selector: selector.to_string() })
        }
        [verb, channel] if eq(verb, "archive") => {
            let channel_ref = channel.trim();
            if !channel_ref.starts_with(crate::channel_ref::CHANNEL_PATH_PREFIX) {
                anyhow::bail!("archive requires a full channel path such as #mosaico/old");
            }
            Ok(ManagementCommand::Archive {
                channel_ref: channel_ref.to_string(),
            })
        }
        [] => anyhow::bail!("empty management command"),
        _ => anyhow::bail!(
            "unsupported management command; supported: add agent[@backend], list agents, list sessions, list all sessions, kill <npub|hex|handle>, archive #root/channel"
        ),
    }
}

/// True when `body` *looks like* a management command: its first meaningful word
/// (after stripping leading inline @mentions) is one of the known command verbs.
///
/// This is the gate that breaks the #375 reply feedback loop. An ordinary
/// conversational reply that merely p-tags the backend is NOT command-shaped, so
/// it is never routed to the management handler and never draws a "mgmt error"
/// reply. A command-shaped body that then fails [`parse_command`] still gets one
/// explicit error, but the agent's prose follow-up is not command-shaped and so
/// cannot re-enter the loop.
pub(super) fn is_command_shaped(body: &str) -> bool {
    let stripped = strip_leading_inline_mentions(body.trim());
    split_words(stripped)
        .ok()
        .and_then(|words| words.into_iter().next())
        .map(|verb| {
            matches!(
                verb.to_ascii_lowercase().as_str(),
                "add" | "list" | "kill" | "archive"
            )
        })
        .unwrap_or(false)
}

/// Whitespace-split with single/double quotes, without shell comment semantics.
/// `#root/child` must remain a word so archive (and similar) can take hash paths.
fn split_words(input: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '\\' if !in_single => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            _ => cur.push(c),
        }
    }
    if in_single || in_double {
        anyhow::bail!("unclosed quote in management command");
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn strip_leading_inline_mentions(mut body: &str) -> &str {
    loop {
        let trimmed = body.trim_start();
        let Some((word, rest_start)) = first_word(trimmed) else {
            return trimmed;
        };
        if !is_inline_mention(word) {
            return trimmed;
        }
        body = &trimmed[rest_start..];
    }
}

fn is_inline_mention(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    lower.starts_with("nostr:npub1")
        || lower.starts_with("nostr:nprofile1")
        || lower.starts_with("npub1")
        || lower.starts_with("nprofile1")
}

fn first_word(body: &str) -> Option<(&str, usize)> {
    if body.is_empty() {
        return None;
    }
    for (idx, ch) in body.char_indices() {
        if ch.is_whitespace() {
            return Some((&body[..idx], idx));
        }
    }
    Some((body, body.len()))
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    // #375: the command-shape gate is what keeps ordinary prose replies (which
    // p-tag the backend) out of the management handler, closing the reply loop.
    #[test]
    fn prose_replies_are_not_command_shaped() {
        // The exact shape of the observed loop: an agent replying to a mgmt error.
        assert!(!is_command_shaped("Acknowledged, no action needed."));
        assert!(!is_command_shaped(
            "@backend-mgmt thanks, I'll take a look at that."
        ));
        assert!(!is_command_shaped(
            "mgmt error: unsupported management command"
        ));
        assert!(!is_command_shaped(
            "please add the coder agent when you can"
        ));
        assert!(!is_command_shaped(""));
        assert!(!is_command_shaped("   "));
    }

    #[test]
    fn real_commands_are_command_shaped() {
        for cmd in [
            "add coder@laptop",
            "list agents",
            "list sessions",
            "list all sessions",
            "kill quill-codex",
            "archive /mosaico/planning",
            // leading inline npub mention (the rendered p-tag) is stripped:
            "npub1qv7resh7tczrrrgwj2t0pwq5jp9r5t86l73gsnlfldfdsqqle2yqnqjwjs add coder",
            "ADD coder", // case-insensitive verb
        ] {
            assert!(is_command_shaped(cmd), "expected command-shaped: {cmd:?}");
        }
    }

    #[test]
    fn command_shaped_but_invalid_still_gets_an_explicit_error_path() {
        // First word is a verb → command-shaped (so it reaches the handler and
        // draws ONE explicit error), even though parse_command rejects it. The
        // loop is still broken because the agent's prose follow-up is not shaped.
        assert!(is_command_shaped("kill")); // missing public selector
        assert!(parse_command("kill").is_err());
        assert!(parse_command("kill mosaico-abc123").is_err());
        assert!(parse_command("kill $abc123").is_err());
        assert!(is_command_shaped("add coder@")); // malformed spec
        assert!(parse_command("add coder@").is_err());
    }

    #[test]
    fn parse_management_commands() {
        assert_eq!(
            parse_command("add coder@laptop").unwrap(),
            ManagementCommand::Add {
                spec: "coder@laptop".to_string()
            }
        );
        assert_eq!(
            parse_command("add coder").unwrap(),
            ManagementCommand::Add {
                spec: "coder".to_string()
            }
        );
        assert!(parse_command("add coder@").is_err());
        assert_eq!(
            parse_command("list agents").unwrap(),
            ManagementCommand::ListAgents
        );
        assert_eq!(
            parse_command("list sessions").unwrap(),
            ManagementCommand::ListSessions {
                all_channels: false
            }
        );
        assert_eq!(
            parse_command("list all sessions").unwrap(),
            ManagementCommand::ListSessions { all_channels: true }
        );
        assert_eq!(
            parse_command("kill quill-codex").unwrap(),
            ManagementCommand::Kill {
                selector: "quill-codex".to_string()
            }
        );
        assert_eq!(
            parse_command("archive #mosaico/planning").unwrap(),
            ManagementCommand::Archive {
                channel_ref: "#mosaico/planning".to_string()
            }
        );
        assert_eq!(
            parse_command("archive \"#mosaico/planning\"").unwrap(),
            ManagementCommand::Archive {
                channel_ref: "#mosaico/planning".to_string()
            }
        );
        assert_eq!(
            parse_command("nostr:npub1qv7resh7tczrrrgwj2t0pwq5jp9r5t86l73gsnlfldfdsqqle2yqnqjwjs list sessions")
                .unwrap(),
            ManagementCommand::ListSessions {
                all_channels: false
            }
        );
        assert_eq!(
            parse_command(
                "npub1qv7resh7tczrrrgwj2t0pwq5jp9r5t86l73gsnlfldfdsqqle2yqnqjwjs archive #mosaico/planning"
            )
            .unwrap(),
            ManagementCommand::Archive {
                channel_ref: "#mosaico/planning".to_string()
            }
        );
        // Only `#root[/child…]` is a full channel path.
        assert!(parse_command("archive planning").is_err());
        assert!(parse_command("archive /mosaico/planning").is_err());
    }
}
