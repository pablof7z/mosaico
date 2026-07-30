use super::*;
use crate::{
    cli::interactive::session_picker::{
        data::SessionRow, picker::state::PickerState, HomeChoice, SessionChoice,
    },
    session_state::SessionState,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn choice(handle: &str, running: bool, last_seen: u64) -> HomeChoice {
    HomeChoice::Session(SessionChoice {
        row: SessionRow {
            handle: handle.into(),
            running,
            last_seen,
            state: if running {
                SessionState::Working
            } else {
                SessionState::Offline
            },
            ..SessionRow::default()
        },
    })
}

#[test]
fn ctrl_o_expands_history_progressively_and_ctrl_u_narrows_it() {
    let now = crate::util::now_secs();
    let hour = 60 * 60;
    let day = 24 * hour;
    let mut state = PickerState::new(
        vec![
            choice("live", true, 0),
            choice("recent", false, now - 40 * 60),
            choice("today", false, now - 8 * hour),
            choice("yesterday", false, now - 20 * hour),
            choice("two-days", false, now - 36 * hour),
            choice("this-week", false, now - 6 * day),
            choice("this-month", false, now - 20 * day),
            choice("older", false, now - 40 * day),
            choice("unknown", false, 0),
        ],
        None,
    );

    assert_eq!(state.visible, vec![0]);
    let expected = [
        (HistoryRange::Hours3, 2),
        (HistoryRange::Hours12, 3),
        (HistoryRange::Day1, 4),
        (HistoryRange::Days2, 5),
        (HistoryRange::Week1, 6),
        (HistoryRange::Days30, 7),
        (HistoryRange::All, 9),
    ];
    for (range, visible) in expected {
        state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL), 10);
        assert_eq!(state.range, range);
        assert_eq!(state.visible.len(), visible);
    }

    state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE), 10);
    assert_eq!(state.range, HistoryRange::All);
    assert_eq!(state.query, "o");
    state.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), 10);
    assert_eq!(state.range, HistoryRange::Days30);
}
