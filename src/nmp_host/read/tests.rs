use super::*;

fn source(status: SourceStatus, reconciled_through: Option<u64>) -> nmp::SourceEvidence {
    nmp::SourceEvidence {
        relay: nmp::RelayUrl::parse("wss://relay.example").expect("a valid relay url"),
        access: AccessContext::Public,
        reconciled_through: reconciled_through.map(nostr::Timestamp::from),
        status,
    }
}

fn branch(sources: Vec<nmp::SourceEvidence>) -> Vec<AcquisitionEvidence> {
    vec![AcquisitionEvidence {
        sources,
        shortfall: Vec::new(),
    }]
}

/// The defect this change deletes. Every source is connected with a REQ
/// outstanding and NOT ONE has answered: that is the state at the very
/// start of every read. Mosaico used to accept it as an authoritative
/// empty result after 500ms of silence, which made a slow relay
/// indistinguishable from an empty one.
#[test]
fn nobody_having_answered_yet_never_completes_a_read() {
    let evidence = branch(vec![
        source(SourceStatus::Requesting, None),
        source(SourceStatus::Requesting, None),
    ]);
    assert!(
        !read_complete(&evidence),
        "an outstanding request is not an answer, however long we stare at it"
    );
}

/// The fact that replaces the clock (nmp#1235). Both relays reached end of
/// stored events having proven nothing — a bounded REQ may claim no
/// coverage interval at all — and that is still a complete answer.
#[test]
fn every_source_finishing_completes_a_read_even_with_nothing_proven() {
    let evidence = branch(vec![
        source(SourceStatus::FinishedStoredEvents, None),
        source(SourceStatus::FinishedStoredEvents, None),
    ]);
    assert!(
        read_complete(&evidence),
        "a source that finished answering has answered, watermark or not"
    );
}

/// One relay finishing says nothing about the other. Settlement is
/// per-source and the read waits for all of them.
#[test]
fn one_source_finishing_does_not_complete_a_read() {
    let evidence = branch(vec![
        source(SourceStatus::FinishedStoredEvents, None),
        source(SourceStatus::Requesting, None),
    ]);
    assert!(
        !read_complete(&evidence),
        "one relay's finish must never answer for a relay still streaming"
    );
}

/// The other independent arm: coverage proven across the window ends the
/// read without waiting on the wire.
#[test]
fn proven_watermarks_complete_a_read_while_still_requesting() {
    let evidence = branch(vec![source(SourceStatus::Requesting, Some(10))]);
    assert!(
        read_complete(&evidence),
        "a proven window needs no further delivery"
    );
}

/// Link failures are not answers, and no clock may promote them to one.
#[test]
fn link_failures_never_complete_a_read() {
    for status in [
        SourceStatus::Connecting,
        SourceStatus::Disconnected,
        SourceStatus::AuthDenied,
        SourceStatus::Error,
    ] {
        assert!(
            !read_complete(&branch(vec![source(status, None)])),
            "{status:?} is a failure to answer, not an answer"
        );
    }
}

/// A routing shortfall is never masked by a sibling source that finished.
#[test]
fn a_shortfall_never_completes_a_read() {
    let evidence = vec![AcquisitionEvidence {
        sources: vec![source(SourceStatus::FinishedStoredEvents, Some(10))],
        shortfall: vec![nmp::ShortfallFact::NoResolvedDemand],
    }];
    assert!(
        !read_complete(&evidence),
        "an unrouted atom is missing evidence no relay can supply"
    );
}

/// Evidence is per branch (#1108) and one branch's proof never answers for
/// a sibling that has not finished.
#[test]
fn a_sibling_branch_never_completes_another() {
    let evidence = vec![
        AcquisitionEvidence {
            sources: vec![source(SourceStatus::FinishedStoredEvents, None)],
            shortfall: Vec::new(),
        },
        AcquisitionEvidence {
            sources: vec![source(SourceStatus::Requesting, None)],
            shortfall: Vec::new(),
        },
    ];
    assert!(
        !read_complete(&evidence),
        "each branch must independently satisfy the rule"
    );
}

#[test]
fn bounded_read_termination_distinguishes_settled_coverage_and_timeout() {
    assert_eq!(
        termination_for(
            &branch(vec![source(SourceStatus::FinishedStoredEvents, None)]),
            BoundedReadTermination::TimedOut,
        ),
        BoundedReadTermination::RelaySettled
    );
    assert_eq!(
        termination_for(
            &branch(vec![source(SourceStatus::CoverageSatisfied, Some(10))]),
            BoundedReadTermination::TimedOut,
        ),
        BoundedReadTermination::CoverageProven
    );
    assert_eq!(
        termination_for(
            &branch(vec![source(SourceStatus::Requesting, None)]),
            BoundedReadTermination::TimedOut,
        ),
        BoundedReadTermination::TimedOut
    );
}

#[test]
fn filter_preserves_multiple_indexed_constraints() {
    let filter = filter(
        &[1],
        &["ab".repeat(32)],
        &[('h', "group".into()), ('t', "marker".into())],
    )
    .unwrap();
    assert_eq!(filter.kinds, Some(BTreeSet::from([1])));
    assert!(filter.authors.is_some());
    assert_eq!(filter.tags.len(), 2);
}
