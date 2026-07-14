pub(super) struct RecipientSummary<'a> {
    pub(super) message_id: &'a str,
    pub(super) pubkey: &'a str,
    pub(super) found: bool,
    pub(super) delivered: bool,
    pub(super) pending: bool,
    pub(super) failed_sync: bool,
    pub(super) recipient_count: usize,
}

pub(super) fn summary(input: &RecipientSummary<'_>) -> String {
    if input.failed_sync {
        return format!(
            "message `{}` failed before recipient `{}` could be proven",
            input.message_id, input.pubkey
        );
    }
    if input.delivered {
        return format!(
            "message `{}` was delivered to recipient `{}`",
            input.message_id, input.pubkey
        );
    }
    if input.pending {
        return format!(
            "message `{}` addresses recipient `{}`, delivery pending",
            input.message_id, input.pubkey
        );
    }
    if input.found {
        return format!(
            "message `{}` has recipient `{}`",
            input.message_id, input.pubkey
        );
    }
    if input.recipient_count > 0 {
        format!(
            "message `{}` does not address recipient `{}`",
            input.message_id, input.pubkey
        )
    } else {
        format!(
            "message `{}` has no durable recipient edges",
            input.message_id
        )
    }
}

pub(super) fn reason(
    found: bool,
    delivered: bool,
    pending: bool,
    failed_sync: bool,
    recipient_count: usize,
) -> &'static str {
    if failed_sync {
        return "message row records a failed/rejected sync state";
    }
    if delivered {
        return "message_recipients contains a delivered edge for this recipient";
    }
    if pending {
        return "message_recipients contains the recipient edge, but delivered_at is not set";
    }
    if found {
        return "message_recipients contains the recipient edge";
    }
    if recipient_count > 0 {
        return "message has hydrated recipient edges and this pubkey is absent";
    }
    "recipient edges are not hydrated for this message"
}
