use crate::fabric_context::model::{MemberKind, MemberRow, PresenceRow};

type MemberSemantics<'a> = (
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    u8,
    Option<&'a str>,
    &'a str,
);
type NativeFailureSemantics<'a> = Option<(&'a str, &'a str)>;
type PresenceSemantics<'a> = (
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    NativeFailureSemantics<'a>,
);

pub(super) fn member_semantics(row: Option<&MemberRow>) -> Option<MemberSemantics<'_>> {
    row.map(|row| {
        (
            row.name.as_str(),
            row.host.as_str(),
            row.workspace.as_str(),
            row.branch.as_str(),
            match row.kind {
                MemberKind::Agent => 1,
                MemberKind::Human => 2,
            },
            row.state.map(|state| state.as_str()),
            row.status.as_str(),
        )
    })
}

pub(super) fn presence_semantics(row: Option<&PresenceRow>) -> Option<PresenceSemantics<'_>> {
    row.map(|row| {
        (
            row.name.as_str(),
            row.host.as_str(),
            row.workspace.as_str(),
            row.branch.as_str(),
            row.state.as_str(),
            row.status.as_str(),
            row.native_failure
                .as_ref()
                .map(|failure| (failure.outcome.as_str(), failure.message.as_str())),
        )
    })
}
