//! Advisories appended after assembly: the emitted text and the receipt must
//! stay consistent with each other.

use super::SELF_PK;
use crate::turn_context::TurnContext;

#[test]
fn advisory_updates_the_exact_emitted_text_and_receipt() {
    use crate::reconcile::hook_context::{FrameKind, HookContextReceipt};

    let mut turn = TurnContext {
        text: Some("fabric".into()),
        receipt: HookContextReceipt::new(
            "turn_check",
            SELF_PK,
            1,
            2,
            FrameKind::Unchanged,
            Some("fabric"),
            Vec::new(),
        ),
        transaction_id: 1,
        revision: 1,
    };
    turn.append_advisory("<channel-topology-nudge />", "channel-topology-nudge");

    let text = turn.text.unwrap();
    assert_eq!(text, "fabric\n\n<channel-topology-nudge />");
    assert_eq!(turn.receipt.bytes, text.len());
    assert_eq!(turn.receipt.input_causes, ["channel-topology-nudge"]);
}
