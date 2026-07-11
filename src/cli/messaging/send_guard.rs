//! `channel send` bare-`@role` guard.
//!
//! In the per-session identity model there is no durable, addressable role
//! identity — a live participant is `@sessionCode-agent`, never `@role`. So a
//! message mentioning a bare `@<role>` that names a KNOWN local agent role (but
//! no session handle) is almost always a mistake: the author wanted a fresh
//! session of that role. Fail fast and point at `dispatch`
//! rather than silently publishing a mention that resolves to nobody.

use anyhow::{bail, Result};

pub(super) fn check(message: &str) -> Result<()> {
    let edge_home = crate::config::edge_home();
    let roles: Vec<String> = crate::identity::list_invitable_agents(&edge_home)
        .into_iter()
        .map(|(slug, _, _)| slug)
        .collect();
    if roles.is_empty() {
        return Ok(());
    }
    for mention in crate::idref::extract_mentions(message) {
        // A mention whose label equals a known role slug is a bare `@role`, not
        // a member reference.
        let label = mention.split('@').next().unwrap_or(&mention);
        if roles.iter().any(|role| role == label) {
            bail!(
                "did you mean to launch a new session of {label}? \
                 use tenex-edge dispatch {label} --workspace <workspace> \
                 --channel <channel> --message <task>"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles_from(message: &str, roles: &[&str]) -> Result<()> {
        // Mirror `check`'s matching against an explicit role list (avoids reading
        // the real keystore in a unit test).
        for mention in crate::idref::extract_mentions(message) {
            let label = mention.split('@').next().unwrap_or(&mention);
            if roles.contains(&label) {
                bail!("did you mean to launch a new session of {label}?");
            }
        }
        Ok(())
    }

    #[test]
    fn bare_known_role_is_rejected() {
        let err = roles_from("hey @reviewer take a look", &["reviewer", "planner"]).unwrap_err();
        assert!(err.to_string().contains("reviewer"));
    }

    #[test]
    fn session_handle_member_is_allowed() {
        assert!(roles_from("thanks @bright-otter-042-reviewer", &["reviewer"]).is_ok());
    }

    #[test]
    fn unknown_mention_is_allowed() {
        assert!(roles_from("cc @pablo about this", &["reviewer"]).is_ok());
    }
}
