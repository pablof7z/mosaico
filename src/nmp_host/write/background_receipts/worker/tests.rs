use super::*;
use nostr::EventId;

#[test]
fn accepted_and_signed_are_not_classified_as_terminal() {
    assert!(matches!(
        classify(WriteStatus::Accepted),
        ReceiptProgress::Intermediate
    ));
    assert!(matches!(
        classify(WriteStatus::Signed(EventId::from_slice(&[7; 32]).unwrap())),
        ReceiptProgress::Intermediate
    ));
}

#[test]
fn cancelled_is_terminal_and_explicit() {
    match classify(WriteStatus::Cancelled) {
        ReceiptProgress::Failure(BackgroundWriteTerminalStatus::Cancelled, detail) => {
            assert_eq!(detail, "write was cancelled before signature promotion")
        }
        _ => panic!("cancelled receipt must be terminal"),
    }
}

#[test]
fn failed_is_immediately_terminal_and_keeps_exact_detail() {
    let detail = "fault=latched: Previous I/O error occurred";
    match classify(WriteStatus::Failed(detail.into())) {
        ReceiptProgress::Failure(BackgroundWriteTerminalStatus::Failed, actual) => {
            assert_eq!(actual, detail)
        }
        _ => panic!("failed receipt must be terminal"),
    }
}
