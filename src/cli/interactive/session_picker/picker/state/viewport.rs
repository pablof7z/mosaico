use super::PickerState;
use crate::cli::interactive::session_picker::HomeChoice;

pub(in crate::cli::interactive::session_picker::picker) struct WindowChoice<'a> {
    pub(in crate::cli::interactive::session_picker::picker) position: usize,
    pub(in crate::cli::interactive::session_picker::picker) choice: &'a HomeChoice,
    pub(in crate::cli::interactive::session_picker::picker) header: Option<&'static str>,
}

impl PickerState {
    pub(in crate::cli::interactive::session_picker::picker) fn ensure_visible(
        &mut self,
        lines: usize,
    ) {
        if let Some(projects) = self.project_picker.as_mut() {
            projects.ensure_visible(lines);
            return;
        }
        if lines == 0 || self.visible.is_empty() {
            self.offset = 0;
            return;
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        }
        while self.offset < self.cursor && self.lines_through(self.offset, self.cursor) > lines {
            self.offset += 1;
        }
        while self.offset > 0 && self.lines_through(self.offset - 1, self.cursor) <= lines {
            self.offset -= 1;
        }
    }

    fn lines_through(&self, offset: usize, end: usize) -> usize {
        (offset..=end)
            .map(|position| 2 + usize::from(self.header_at(position, offset).is_some()))
            .sum()
    }

    pub(in crate::cli::interactive::session_picker::picker) fn window(
        &self,
        max_lines: usize,
    ) -> Vec<WindowChoice<'_>> {
        let mut used = 0;
        let mut window = Vec::new();
        for position in self.offset..self.visible.len() {
            let header = self.header_at(position, self.offset);
            let height = 2 + usize::from(header.is_some());
            if !window.is_empty() && used + height > max_lines {
                break;
            }
            used += height;
            window.push(WindowChoice {
                position,
                choice: &self.choices[self.visible[position]],
                header,
            });
        }
        window
    }

    fn header_at(&self, position: usize, offset: usize) -> Option<&'static str> {
        if !self.query.is_empty() {
            return None;
        }
        let current = &self.choices[self.visible[position]];
        let changed = position == offset
            || current.is_session()
                != self.choices[self.visible[position.saturating_sub(1)]].is_session();
        changed.then_some(if current.is_session() {
            "Sessions"
        } else {
            "Start a session"
        })
    }

    pub(in crate::cli::interactive::session_picker::picker) fn counts(&self) -> (usize, usize) {
        self.visible
            .iter()
            .fold((0, 0), |(sessions, agents), &index| {
                if self.choices[index].is_session() {
                    (sessions + 1, agents)
                } else {
                    (sessions, agents + 1)
                }
            })
    }
}
