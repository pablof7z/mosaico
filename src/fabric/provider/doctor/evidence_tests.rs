use std::collections::{BTreeMap, BTreeSet};

use nmp::{
    AccessContext, AcquisitionEvidence, ReceiptResult, RelayState, Row, ShortfallFact,
    SourceEvidence, SourceStatus, WriteOutcome,
};
use nostr::{EventBuilder, Keys, Kind};

use super::*;

fn event() -> nostr::Event {
    EventBuilder::new(Kind::TextNote, "cached")
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

fn source(status: SourceStatus) -> SourceEvidence {
    SourceEvidence {
        relay: nmp::RelayUrl::parse("wss://read.example").unwrap(),
        access: AccessContext::Public,
        reconciled_through: None,
        status,
    }
}

fn read(
    termination: BoundedReadTermination,
    rows: Vec<nostr::Event>,
    status: SourceStatus,
    shortfall: Vec<ShortfallFact>,
) -> BoundedRead {
    let relay = nmp::RelayUrl::parse("wss://read.example").unwrap();
    BoundedRead {
        rows: rows
            .into_iter()
            .map(|event| Row {
                event,
                sources: BTreeSet::from([relay.clone()]),
            })
            .collect(),
        evidence: vec![AcquisitionEvidence {
            sources: vec![source(status)],
            shortfall,
        }],
        termination,
    }
}

#[test]
fn every_relay_must_publish_for_the_write_probe_to_pass() {
    let published = nmp::RelayUrl::parse("wss://published.example").unwrap();
    let rejected = nmp::RelayUrl::parse("wss://rejected.example").unwrap();
    let step = publish(
        &event().id,
        ReceiptResult {
            outcome: WriteOutcome::Settled,
            relays: BTreeMap::from([
                (published, RelayState::Published),
                (
                    rejected,
                    RelayState::Rejected {
                        reason: "blocked".into(),
                    },
                ),
            ]),
        },
    );

    assert_eq!(step.status, ProbeStatus::Failed);
    assert!(step.summary.contains("blocked"));
    assert_eq!(step.relays.len(), 2);
    assert_eq!(step.terminal.as_deref(), Some("Settled"));
}

#[test]
fn authentication_failure_is_named_instead_of_collapsed_to_a_generic_error() {
    let relay = nmp::RelayUrl::parse("wss://protected.example").unwrap();
    let step = publish(
        &event().id,
        ReceiptResult {
            outcome: WriteOutcome::Settled,
            relays: BTreeMap::from([(
                relay,
                RelayState::AuthFailed {
                    reason: "challenge rejected".into(),
                    pubkey: Keys::generate().public_key(),
                    source: nmp::AuthDenialSource::Relay,
                },
            )]),
        },
    );

    assert_eq!(step.status, ProbeStatus::Failed);
    assert!(step.summary.contains("authentication failed"));
    assert!(step.summary.contains("challenge rejected"));
}

#[test]
fn all_relays_published_is_the_only_verified_write_terminal() {
    let result = ReceiptResult {
        outcome: WriteOutcome::Settled,
        relays: ["wss://one.example", "wss://two.example"]
            .into_iter()
            .map(|relay| (nmp::RelayUrl::parse(relay).unwrap(), RelayState::Published))
            .collect(),
    };

    let step = publish(&event().id, result);
    assert_eq!(step.status, ProbeStatus::Verified);
    assert_eq!(step.relays.len(), 2);
}

#[test]
fn cached_rows_that_time_out_are_not_current_relay_readback() {
    let step = readback(
        read(
            BoundedReadTermination::TimedOut,
            vec![event()],
            SourceStatus::Requesting,
            Vec::new(),
        ),
        true,
        "doctor marker",
    );

    assert_eq!(step.status, ProbeStatus::Failed);
    assert!(step.summary.contains("TimedOut"));
    assert!(step.summary.contains("1 cached/current event"));
}

#[test]
fn coverage_proven_cache_is_not_current_relay_readback() {
    let step = readback(
        read(
            BoundedReadTermination::CoverageProven,
            vec![event()],
            SourceStatus::CoverageSatisfied,
            Vec::new(),
        ),
        true,
        "doctor marker",
    );

    assert_eq!(step.status, ProbeStatus::Failed);
    assert!(step.summary.contains("CoverageProven"));
}

#[test]
fn relay_settled_empty_is_a_real_empty_answer_but_not_a_marker_readback() {
    let empty = read(
        BoundedReadTermination::RelaySettled,
        Vec::new(),
        SourceStatus::FinishedStoredEvents,
        Vec::new(),
    );
    assert_eq!(
        readback(empty.clone(), false, "group metadata").status,
        ProbeStatus::Verified
    );
    assert_eq!(
        readback(empty, true, "doctor marker").status,
        ProbeStatus::Failed
    );
}

#[test]
fn disconnected_and_shortfall_evidence_survive_in_the_report() {
    let step = readback(
        read(
            BoundedReadTermination::SubscriptionClosed,
            Vec::new(),
            SourceStatus::Disconnected,
            vec![ShortfallFact::NoResolvedDemand],
        ),
        false,
        "group metadata",
    );
    let acquisition = step.acquisition.expect("acquisition details");

    assert_eq!(step.status, ProbeStatus::Failed);
    assert_eq!(acquisition.branches.len(), 1);
    assert_eq!(acquisition.branches[0].sources[0].status, "Disconnected");
    assert_eq!(acquisition.branches[0].shortfalls, vec!["NoResolvedDemand"]);
}
