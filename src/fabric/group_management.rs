#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupPublishOutcome {
    Applied,
    Retryable,
    Rejected,
}

impl GroupPublishOutcome {
    pub(crate) fn is_applied(self) -> bool {
        matches!(self, Self::Applied)
    }

    pub(crate) fn is_rejected(self) -> bool {
        matches!(self, Self::Rejected)
    }
}

pub(crate) fn classify_group_publish_error(reason: &str) -> GroupPublishOutcome {
    let lower = reason.to_ascii_lowercase();
    if is_benign_duplicate(&lower) {
        return GroupPublishOutcome::Applied;
    }
    if is_permanent_rejection(&lower) {
        return GroupPublishOutcome::Rejected;
    }
    if is_retryable_rejection(&lower) {
        return GroupPublishOutcome::Retryable;
    }
    GroupPublishOutcome::Rejected
}

fn is_benign_duplicate(lower: &str) -> bool {
    lower.contains("already exists")
        || lower.contains("duplicate")
        || lower.contains("members already")
        || lower.contains("already a member")
        || lower.contains("all targets are members already")
}

fn is_permanent_rejection(lower: &str) -> bool {
    lower.contains("kind 9000 is not allowed")
        || lower.contains("kind 9001 is not allowed")
        || lower.contains("kind 9002 is not allowed")
        || lower.contains("kind 9007 is not allowed")
        || lower.contains("invalid moderation action")
        || lower.contains("missing metadata tags")
}

fn is_retryable_rejection(lower: &str) -> bool {
    lower.contains("timeout")
        || lower.contains("relay not connected")
        || lower.contains("not connected")
        || lower.contains("can't send message to the 'nostr' channel")
        || lower.contains("cannot send message to the 'nostr' channel")
        || lower.contains("doesn't exist")
        || lower.contains("does not exist")
}

#[cfg(test)]
mod tests {
    use super::{classify_group_publish_error, GroupPublishOutcome};

    #[test]
    fn benign_member_duplicate_counts_as_applied() {
        let outcome = classify_group_publish_error(
            "relay rejected event: blocked: all targets are members already",
        );
        assert_eq!(outcome, GroupPublishOutcome::Applied);
    }

    #[test]
    fn permanent_rejection_wins_over_timeout() {
        let outcome = classify_group_publish_error(
            "relay rejected event: timeout; blocked: kind 9002 is not allowed",
        );
        assert_eq!(outcome, GroupPublishOutcome::Rejected);
    }

    #[test]
    fn transport_failures_are_retryable() {
        let outcome = classify_group_publish_error(
            "relay rejected event: can't send message to the 'nostr' channel; timeout",
        );
        assert_eq!(outcome, GroupPublishOutcome::Retryable);
    }
}
