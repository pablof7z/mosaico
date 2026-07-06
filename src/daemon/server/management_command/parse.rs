//! Parser for backend-addressed management commands.

use super::ManagementCommand;
use anyhow::{Context, Result};

const HASH_PLACEHOLDER: &str = "__tenex_edge_channel_hash__";

pub(super) fn parse_command(body: &str) -> Result<ManagementCommand> {
    let body = strip_leading_inline_mentions(body.trim());
    let body = preserve_archive_channel_hash(body);
    let words = shlex::split(&body).context("could not parse command quoting")?;
    match words.as_slice() {
        [verb, spec] if eq(verb, "add") => {
            let target = crate::idref::parse_agent_backend_ref(spec)
                .with_context(|| format!("malformed agent spec {spec:?}"))?;
            if target.backend.is_none() {
                anyhow::bail!("add requires agent@backend");
            }
            Ok(ManagementCommand::Add { spec: spec.clone() })
        }
        [a, b] if eq(a, "list") && eq(b, "sessions") => {
            Ok(ManagementCommand::ListSessions {
                all_channels: false,
            })
        }
        [a, b, c] if eq(a, "list") && eq(b, "all") && eq(c, "sessions") => {
            Ok(ManagementCommand::ListSessions { all_channels: true })
        }
        [verb, id] if eq(verb, "kill") => {
            let session_id = id.strip_prefix('$').unwrap_or(id).trim();
            if session_id.is_empty() {
                anyhow::bail!("kill requires a session id");
            }
            Ok(ManagementCommand::Kill {
                session_id: session_id.to_string(),
            })
        }
        [verb, channel] if eq(verb, "archive") => {
            let channel = channel
                .strip_prefix(HASH_PLACEHOLDER)
                .map(|rest| format!("#{rest}"))
                .unwrap_or_else(|| channel.clone());
            let channel_ref = channel.strip_prefix('#').unwrap_or(&channel).trim();
            if channel_ref.is_empty() {
                anyhow::bail!("archive requires a channel name");
            }
            Ok(ManagementCommand::Archive {
                channel_ref: channel_ref.to_string(),
            })
        }
        [] => anyhow::bail!("empty management command"),
        _ => anyhow::bail!(
            "unsupported management command; supported: add agent@backend, list sessions, list all sessions, kill <session-id>, archive #channel"
        ),
    }
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

fn preserve_archive_channel_hash(body: &str) -> String {
    let Some((verb, rest_start)) = first_word(body) else {
        return body.to_string();
    };
    if !eq(verb, "archive") {
        return body.to_string();
    }

    let rest = &body[rest_start..];
    let whitespace_len = rest.len() - rest.trim_start().len();
    let channel = &rest[whitespace_len..];
    let Some(after_hash) = channel.strip_prefix('#') else {
        return body.to_string();
    };

    let mut out = String::with_capacity(body.len() + HASH_PLACEHOLDER.len());
    out.push_str(&body[..rest_start + whitespace_len]);
    out.push_str(HASH_PLACEHOLDER);
    out.push_str(after_hash);
    out
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

    #[test]
    fn parse_management_commands() {
        assert_eq!(
            parse_command("add coder@laptop").unwrap(),
            ManagementCommand::Add {
                spec: "coder@laptop".to_string()
            }
        );
        assert!(parse_command("add coder").is_err());
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
            parse_command("kill $abc123").unwrap(),
            ManagementCommand::Kill {
                session_id: "abc123".to_string()
            }
        );
        assert_eq!(
            parse_command("archive #planning").unwrap(),
            ManagementCommand::Archive {
                channel_ref: "planning".to_string()
            }
        );
        assert_eq!(
            parse_command("archive \"#planning\"").unwrap(),
            ManagementCommand::Archive {
                channel_ref: "planning".to_string()
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
                "npub1qv7resh7tczrrrgwj2t0pwq5jp9r5t86l73gsnlfldfdsqqle2yqnqjwjs archive #planning"
            )
            .unwrap(),
            ManagementCommand::Archive {
                channel_ref: "planning".to_string()
            }
        );
    }
}
