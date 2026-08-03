//! Reading the subjects out of a NIP-29 roster snapshot (kind:39001/39002).
//!
//! # Placeholder, and deliberately labelled as one
//!
//! This belongs in NMP and is not there yet. `nmp-nip29` exposes the three kind
//! constants and two evidence PREDICATES — `member_list_includes_at` /
//! `admin_list_includes_at`, which lower to a `Binding` and answer "does this
//! list name X" without ever parsing a list — and nothing that hands back the
//! subjects themselves. That is a documented refusal, not an omission: NMP is
//! careful that inclusion in an observed list is EVIDENCE, and absence from one
//! is not evidence of non-membership, so it declines to mint a value that would
//! read as exact current state.
//!
//! Mosaico still needs the subjects, so this parses them — in ONE place. It
//! existed in two, which is the actual defect: `fetch_group_state` read the
//! role at `p[2]`, and the materializer's own copy dropped it, so the same
//! event described a different roster depending on which caller decoded it.
//!
//! Do not grow this into a roster model. When NMP ships a reader, this file is
//! deleted and its two callers move over.

use nostr::Event;

/// One `p` row of a relay-signed roster snapshot: the subject, and the role
/// the list assigned it if it assigned one.
///
/// `["p", "<pubkey>"]` and `["p", "<pubkey>", "<role>"]` are both well-formed;
/// the role is optional in NIP-29 and `None` means the list said nothing, which
/// is not the same as saying "member".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RosterSubject {
    pub(crate) pubkey: String,
    pub(crate) role: Option<String>,
}

/// Every `p` row of a roster snapshot, in the order the event lists them.
///
/// Kind-blind on purpose: 39001 and 39002 carry the same row shape and differ
/// only in what the relay means by the list. Deciding that is the caller's.
pub(crate) fn subjects(event: &Event) -> Vec<RosterSubject> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let row = tag.as_slice();
            if row.first().map(String::as_str) != Some("p") {
                return None;
            }
            let pubkey = row.get(1).filter(|value| !value.is_empty())?.clone();
            Ok::<_, ()>(RosterSubject {
                pubkey,
                role: row.get(2).filter(|value| !value.is_empty()).cloned(),
            })
            .ok()
        })
        .collect()
}

/// Just the subject pubkeys, for a caller that stores membership without role.
pub(crate) fn subject_pubkeys(event: &Event) -> Vec<String> {
    subjects(event)
        .into_iter()
        .map(|subject| subject.pubkey)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn roster(rows: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(39001u16), "")
            .tags(
                rows.iter()
                    .map(|row| Tag::parse(row.iter().copied()).unwrap()),
            )
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    /// The divergence this file removes: one caller kept the role, the other
    /// dropped it, so the same event described two different rosters.
    #[test]
    fn one_parse_serves_both_the_role_keeping_and_the_role_dropping_caller() {
        let event = roster(&[
            &["d", "room"],
            &["p", "aa", "admin"],
            &["p", "bb"],
            &["p", "cc", ""],
        ]);

        assert_eq!(
            subjects(&event),
            vec![
                RosterSubject {
                    pubkey: "aa".into(),
                    role: Some("admin".into())
                },
                RosterSubject {
                    pubkey: "bb".into(),
                    role: None
                },
                RosterSubject {
                    pubkey: "cc".into(),
                    role: None
                },
            ]
        );
        assert_eq!(subject_pubkeys(&event), vec!["aa", "bb", "cc"]);
    }

    #[test]
    fn rows_that_name_no_subject_are_not_subjects() {
        let event = roster(&[&["d", "room"], &["p"], &["p", ""], &["e", "aa"]]);
        assert!(subjects(&event).is_empty());
    }
}
