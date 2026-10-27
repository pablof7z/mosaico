use crate::util::format_local_datetime;

/// Everything needed to render one inbound message as an email-like envelope.
pub(in crate::cli) struct EnvelopeView<'a> {
    pub from_slug: &'a str,
    /// Optional public sender handle; never a private runtime locator.
    pub from_session: &'a str,
    pub host: &'a str,
    pub self_host: &'a str,
    pub subject: &'a str,
    pub branch: &'a str,
    pub commit: &'a str,
    pub dirty: u32,
    pub id: &'a str,
    pub sent_at: u64,
    pub now: u64,
    pub body: &'a str,
}

/// Render an inbound message as an email-like envelope. Optional subject and
/// branch lines are omitted.
pub(in crate::cli) fn format_envelope(e: &EnvelopeView) -> String {
    use crate::util::dirty_label;
    use std::fmt::Write as _;

    let host = if e.host.is_empty() {
        e.self_host
    } else {
        e.host
    };
    let from = if e.from_session.is_empty() {
        crate::idref::agent_label(e.from_slug, host)
    } else {
        crate::idref::session_label(e.from_slug, host)
    };

    let mut output = String::new();
    let _ = write!(output, "From: {from}");
    let _ = write!(
        output,
        "\nDate: {} ({})",
        format_local_datetime(e.sent_at),
        crate::util::relative_time(e.sent_at, e.now)
    );
    if !e.subject.is_empty() {
        let _ = write!(output, "\nSubject: {}", e.subject);
    }
    if !e.branch.is_empty() {
        let commit = if e.commit.is_empty() {
            String::new()
        } else {
            format!(" ({})", e.commit)
        };
        let _ = write!(
            output,
            "\nBranch: {}{}{}",
            e.branch,
            commit,
            dirty_label(e.dirty)
        );
    }
    let _ = write!(output, "\nID: {}", e.id);
    let _ = write!(output, "\n--\n{}", e.body);
    output
}
