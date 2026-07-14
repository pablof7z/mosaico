pub(in crate::daemon::server::probe::validate) struct RecipientTarget {
    pub(super) message_prefix: String,
    pub(super) recipient_pubkey: String,
}

pub(in crate::daemon::server::probe::validate) fn recipient_target(
    target: &str,
) -> Option<RecipientTarget> {
    colon_target(target, "recipient:")
        .or_else(|| colon_target(target, "delivery:"))
        .or_else(|| path_target(target, "recipient/"))
        .or_else(|| path_target(target, "delivery/"))
}

fn colon_target(target: &str, prefix: &str) -> Option<RecipientTarget> {
    exact_target(target.strip_prefix(prefix)?, ':')
}

fn path_target(target: &str, prefix: &str) -> Option<RecipientTarget> {
    exact_target(target.strip_prefix(prefix)?, '/')
}

fn exact_target(rest: &str, separator: char) -> Option<RecipientTarget> {
    let mut parts = rest.split(separator);
    let message_prefix = parts.next()?;
    let recipient_pubkey = parts.next()?;
    if parts.next().is_some()
        || message_prefix.trim().is_empty()
        || recipient_pubkey.trim().is_empty()
    {
        return None;
    }
    Some(RecipientTarget {
        message_prefix: message_prefix.to_string(),
        recipient_pubkey: recipient_pubkey.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipient_target_rejects_removed_runtime_suffix() {
        assert!(recipient_target("recipient:event:pubkey:runtime").is_none());
        assert!(recipient_target("recipient/event/pubkey/runtime").is_none());
    }
}
