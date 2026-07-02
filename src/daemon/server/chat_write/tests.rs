use super::*;
use crate::util::CHAT_RENDER_WORD_LIMIT;

fn message(words: usize) -> String {
    (0..words)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn long_message_guard_requires_explicit_override() {
    let long = ChatWriteParams {
        message: message(CHAT_RENDER_WORD_LIMIT + 1),
        long_message: false,
        ..Default::default()
    };
    assert!(long_message_requires_override(&long));

    let allowed = ChatWriteParams {
        long_message: true,
        ..long
    };
    assert!(!long_message_requires_override(&allowed));

    let short = ChatWriteParams {
        message: message(CHAT_RENDER_WORD_LIMIT),
        long_message: false,
        ..Default::default()
    };
    assert!(!long_message_requires_override(&short));
}
