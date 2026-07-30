use portable_pty::ExitStatus;
use std::collections::VecDeque;

const DIAGNOSTIC_LIMIT: usize = 8 * 1024;
const TERMINAL_RESTORE: &str = "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\
    \x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?2004l\x1b[?1016l\x1b[?1007l\
    \x1b[?2026l\x1b[?1049l\x1b[?1047l\x1b[?47l\x1b[?25h\x1b[>4;0m\x1b[<u";

pub(super) fn plain_tail(bytes: &VecDeque<u8>) -> String {
    let visible = strip_terminal_controls(bytes.iter().copied());
    let text = String::from_utf8_lossy(&visible);
    let mut lines = VecDeque::<String>::new();
    let mut size = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || lines.back().is_some_and(|previous| previous == line) {
            continue;
        }
        let line = tail_at_char_boundary(line, DIAGNOSTIC_LIMIT).to_string();
        if !lines.is_empty() {
            size += 1;
        }
        size += line.len();
        lines.push_back(line);
        while size > DIAGNOSTIC_LIMIT {
            let Some(removed) = lines.pop_front() else {
                break;
            };
            size = size.saturating_sub(removed.len());
            if !lines.is_empty() {
                size = size.saturating_sub(1);
            }
        }
    }
    lines.into_iter().collect::<Vec<_>>().join("\n")
}

fn tail_at_char_boundary(text: &str, limit: usize) -> &str {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

pub(super) fn failure_footer(
    agent: &str,
    command: &[String],
    status: &ExitStatus,
    diagnostic_tail: &str,
) -> Option<Vec<u8>> {
    if status.success() {
        return None;
    }
    let code = status.exit_code();
    let output = if diagnostic_tail.is_empty() {
        "(no terminal output captured)"
    } else {
        diagnostic_tail
    };
    Some(
        format!(
            "{TERMINAL_RESTORE}\r\n\r\nmosaico: {agent} exited with code {code}\r\n\
             command: {command:?}\r\nlast terminal output:\r\n{output}\r\n"
        )
        .into_bytes(),
    )
}

#[derive(Clone, Copy)]
enum EscapeState {
    Text,
    Escape,
    Csi,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

fn strip_terminal_controls(bytes: impl IntoIterator<Item = u8>) -> Vec<u8> {
    use EscapeState::*;

    let mut state = Text;
    let mut visible = Vec::new();
    for byte in bytes {
        state = match state {
            Text => match byte {
                0x1b => Escape,
                0x9b => Csi,
                0x9d => Osc,
                b'\r' | b'\n' => {
                    if visible.last() != Some(&b'\n') {
                        visible.push(b'\n');
                    }
                    Text
                }
                b'\t' => {
                    visible.push(b' ');
                    Text
                }
                0x08 => {
                    if visible.last().is_some_and(|last| *last != b'\n') {
                        visible.pop();
                    }
                    Text
                }
                0x00..=0x1f | 0x7f => Text,
                _ => {
                    visible.push(byte);
                    Text
                }
            },
            Escape => match byte {
                b'[' => Csi,
                b']' => Osc,
                b'P' | b'X' | b'^' | b'_' => String,
                _ => Text,
            },
            Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    Text
                } else {
                    Csi
                }
            }
            Osc => match byte {
                0x07 => Text,
                0x1b => OscEscape,
                _ => Osc,
            },
            OscEscape => {
                if byte == b'\\' {
                    Text
                } else {
                    Osc
                }
            }
            String => {
                if byte == 0x1b {
                    StringEscape
                } else {
                    String
                }
            }
            StringEscape => {
                if byte == b'\\' {
                    Text
                } else {
                    String
                }
            }
        };
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_tail_keeps_errors_without_control_sequences() {
        let bytes = VecDeque::from(
            b"\x1b[?1049h\x1b]0;codex\x07error: bad config\r\n\
              \x1b[31munexpected argument '--old'\x1b[0m\r\n"
                .to_vec(),
        );

        assert_eq!(
            plain_tail(&bytes),
            "error: bad config\nunexpected argument '--old'"
        );
    }

    #[test]
    fn terminal_tail_retains_the_end_of_one_oversized_line() {
        let mut raw = vec![b'x'; DIAGNOSTIC_LIMIT + 100];
        raw.extend_from_slice(b" exact failure");
        let tail = plain_tail(&VecDeque::from(raw));

        assert!(tail.len() <= DIAGNOSTIC_LIMIT);
        assert!(tail.ends_with("exact failure"));
    }

    #[test]
    fn failure_footer_appears_only_for_unsuccessful_exits() {
        let failed = ExitStatus::with_exit_code(23);
        let footer = String::from_utf8(
            failure_footer(
                "codex",
                &["codex".into(), "--bad".into()],
                &failed,
                "bad flag",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(footer.contains("codex exited with code 23"));
        assert!(footer.contains("[\"codex\", \"--bad\"]"));
        assert!(footer.contains("bad flag"));

        let success = ExitStatus::with_exit_code(0);
        assert!(failure_footer("codex", &["codex".into()], &success, "").is_none());
    }
}
