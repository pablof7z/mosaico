use anyhow::{bail, Result};

pub(super) fn check(message: &str, tags: &[String], force: bool) -> Result<()> {
    if force || !tags.is_empty() {
        return Ok(());
    }
    let Some(mention) = first_mention_attempt(message) else {
        return Ok(());
    };
    bail!(
        "message contains @{mention} but no --tag; did you mean to mention {mention}? \
         Use `--tag {mention}`. If not, publish the literal text with `--force`"
    )
}

fn first_mention_attempt(message: &str) -> Option<&str> {
    for (at, _) in message.match_indices('@') {
        if message[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        let after = &message[at + 1..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@')))
            .unwrap_or(after.len());
        let candidate = after[..end].trim_end_matches(['.', '@']);
        if !candidate.is_empty() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untagged_mention_attempt_explains_tag_and_force_paths() {
        let error = check("hello @agent1", &[], false).unwrap_err().to_string();

        assert!(error.contains("did you mean to mention agent1?"));
        assert!(error.contains("--tag agent1"));
        assert!(error.contains("--force"));
    }

    #[test]
    fn force_allows_literal_mention_text() {
        assert!(check("hello @agent1", &[], true).is_ok());
    }

    #[test]
    fn any_tag_bypasses_every_inline_mention_check() {
        assert!(check(
            "hello, @a2 keeps ignoring me today",
            &["a1".to_string()],
            false
        )
        .is_ok());
    }

    #[test]
    fn email_addresses_are_not_mention_attempts() {
        assert!(check("email dev@example.com", &[], false).is_ok());
    }
}
