use super::data::SessionRow;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const GAP: &str = "  ";
const FULL_MIN_WIDTH: usize = 75;
const COMPACT_MIN_WIDTH: usize = 44;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Density {
    Full,
    Compact,
    Narrow,
}

#[derive(Clone, Debug)]
pub(super) struct SessionLayout {
    width: usize,
    density: Density,
    handle_width: usize,
    scope_width: usize,
    work_width: usize,
}

impl SessionLayout {
    pub(super) fn new(rows: &[SessionRow], width: usize) -> Self {
        let width = width.max(1);
        let desired_handle = rows
            .iter()
            .map(|row| text_width(&format!("@{}", row.handle)))
            .max()
            .unwrap_or(0)
            .max(text_width("SESSION"));

        if width >= FULL_MIN_WIDTH {
            return Self::full(rows, width, desired_handle);
        }
        if width >= COMPACT_MIN_WIDTH {
            return Self::compact(width, desired_handle);
        }
        Self::narrow(width, desired_handle)
    }

    fn full(rows: &[SessionRow], width: usize, desired_handle: usize) -> Self {
        let handle_width = desired_handle.clamp(16, if width < 88 { 18 } else { 24 });
        let fixed_width = handle_width + 7 + 10 + GAP.len() * 4;
        let content_width = width.saturating_sub(fixed_width);
        let desired_scope = rows
            .iter()
            .map(SessionRow::scope)
            .map(|scope| text_width(&scope))
            .max()
            .unwrap_or(0)
            .max(text_width("WORKSPACE / CHANNEL"));
        let scope_width = desired_scope
            .clamp(16, 32)
            .min((content_width * 2 / 5).max(16));
        let work_width = content_width.saturating_sub(scope_width);
        Self {
            width,
            density: Density::Full,
            handle_width,
            scope_width,
            work_width,
        }
    }

    fn compact(width: usize, desired_handle: usize) -> Self {
        let handle_width = desired_handle.clamp(16, 24).min(width - 27);
        let work_width = width - handle_width - 7 - GAP.len() * 2;
        Self {
            width,
            density: Density::Compact,
            handle_width,
            scope_width: 0,
            work_width,
        }
    }

    fn narrow(width: usize, desired_handle: usize) -> Self {
        let handle_width = if width <= GAP.len() + 1 {
            width
        } else {
            desired_handle
                .clamp(8, 16)
                .min(width.saturating_sub(GAP.len() + 8).max(1))
        };
        let work_width = width.saturating_sub(handle_width + GAP.len());
        Self {
            width,
            density: Density::Narrow,
            handle_width,
            scope_width: 0,
            work_width,
        }
    }

    pub(super) fn header(&self) -> String {
        match self.density {
            Density::Full => self.columns(
                "SESSION",
                "STATE",
                Some("WORKSPACE / CHANNEL"),
                Some("UPDATED"),
                "CURRENT WORK",
            ),
            Density::Compact => self.columns("SESSION", "STATE", None, None, "CURRENT WORK"),
            Density::Narrow => join_cells(&[
                cell("SESSION", self.handle_width),
                cell("STATE / WORK", self.work_width),
            ]),
        }
    }

    pub(super) fn row(&self, row: &SessionRow, now: u64) -> String {
        let handle = format!("@{}", row.handle);
        let state = if row.busy { "working" } else { "idle" };
        match self.density {
            Density::Full => self.columns(
                &handle,
                state,
                Some(&row.scope()),
                Some(&row.seen(now)),
                &row.work(),
            ),
            Density::Compact => self.columns(&handle, state, None, None, &row.work()),
            Density::Narrow => join_cells(&[
                cell(&handle, self.handle_width),
                cell(&format!("{state} · {}", row.work()), self.work_width),
            ]),
        }
    }

    fn columns(
        &self,
        handle: &str,
        state: &str,
        scope: Option<&str>,
        seen: Option<&str>,
        work: &str,
    ) -> String {
        let mut cells = vec![cell(handle, self.handle_width), cell(state, 7)];
        if let Some(scope) = scope {
            cells.push(cell(scope, self.scope_width));
        }
        if let Some(seen) = seen {
            cells.push(cell(seen, 10));
        }
        cells.push(cell(work, self.work_width));
        let rendered = join_cells(&cells);
        debug_assert_eq!(text_width(&rendered), self.width);
        rendered
    }
}

impl SessionRow {
    fn scope(&self) -> String {
        match (self.workspace.is_empty(), self.channels.as_slice()) {
            (true, _) => "(no workspace)".to_string(),
            (false, []) => self.workspace.clone(),
            (false, [channel]) if channel == &self.workspace => self.workspace.clone(),
            (false, _) => format!("{}/{}", self.workspace, self.channels.join(",")),
        }
    }

    fn seen(&self, now: u64) -> String {
        if self.last_seen == 0 {
            "unknown".to_string()
        } else {
            crate::util::relative_time(self.last_seen, now)
        }
    }

    fn work(&self) -> String {
        let title = self.title.trim();
        let title = if title.is_empty() {
            "(untitled)"
        } else {
            title
        };
        let activity = self.activity.trim();
        if activity.is_empty() || activity == title || title == "(untitled)" {
            title.to_string()
        } else {
            format!("{title} — {activity}")
        }
    }
}

pub(super) fn picker_page_size(session_count: usize, terminal_height: usize) -> usize {
    session_count.min((terminal_height / 2).max(3))
}

fn join_cells(cells: &[String]) -> String {
    if cells.last().is_some_and(String::is_empty) {
        return cells.first().cloned().unwrap_or_default();
    }
    cells.join(GAP)
}

fn cell(value: &str, width: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let content = if text_width(&normalized) > width {
        clipped(&normalized, width)
    } else {
        normalized
    };
    let padding = width.saturating_sub(text_width(&content));
    format!("{content}{}", " ".repeat(padding))
}

fn clipped(value: &str, width: usize) -> String {
    let suffix = usize::from(width > 1);
    let content_width = width.saturating_sub(suffix);
    let mut rendered = String::new();
    let mut used = 0;
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > content_width {
            break;
        }
        rendered.push(character);
        used += character_width;
    }
    if suffix == 1 {
        rendered.push('…');
    }
    rendered
}

fn text_width(value: &str) -> usize {
    value.width()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> SessionRow {
        SessionRow {
            handle: "delta-codex".into(),
            workspace: "tenex-edge".into(),
            channels: vec!["suspended-sessions".into()],
            title: "Implement suspended session state".into(),
            activity: "running focused tests".into(),
            busy: true,
            last_seen: 98,
            ..SessionRow::default()
        }
    }

    #[test]
    fn wide_layout_has_aligned_named_columns_and_one_line_rows() {
        let row = row();
        let layout = SessionLayout::new(std::slice::from_ref(&row), 120);
        let header = layout.header();
        let rendered = layout.row(&row, 100);

        assert_eq!(text_width(&header), 120);
        assert_eq!(text_width(&rendered), 120);
        assert_eq!(header.find("STATE"), rendered.find("working"));
        assert_eq!(header.find("WORKSPACE"), rendered.find("tenex-edge"));
        assert_eq!(header.find("UPDATED"), rendered.find("just now"));
        assert!(!rendered.contains('\n'));
    }

    #[test]
    fn compact_and_narrow_layouts_keep_identity_state_and_work() {
        let row = row();
        for width in [60, 36] {
            let layout = SessionLayout::new(std::slice::from_ref(&row), width);
            let rendered = layout.row(&row, 100);
            assert_eq!(text_width(&rendered), width);
            assert!(rendered.contains("@delta"));
            assert!(rendered.contains("work"));
        }
    }

    #[test]
    fn cells_normalize_and_truncate_without_exceeding_width() {
        assert_eq!(cell("one\n two", 10), "one two   ");
        assert_eq!(cell("abcdefgh", 5), "abcd…");
        assert_eq!(cell("界界界", 5), "界界…");
        assert_eq!(cell("abcdefgh", 1), "a");
    }

    #[test]
    fn picker_uses_half_the_terminal_height_and_caps_at_session_count() {
        assert_eq!(picker_page_size(100, 50), 25);
        assert_eq!(picker_page_size(10, 50), 10);
        assert_eq!(picker_page_size(100, 10), 5);
    }
}
