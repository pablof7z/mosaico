use super::PickerState;
use crate::cli::interactive::session_picker::HomeChoice;

pub(in crate::cli::interactive::session_picker::picker) struct WindowChoice<'a> {
    pub(in crate::cli::interactive::session_picker::picker) position: usize,
    pub(in crate::cli::interactive::session_picker::picker) choice: &'a HomeChoice,
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
        (end - offset + 1) * 2
    }

    pub(in crate::cli::interactive::session_picker::picker) fn window(
        &self,
        max_lines: usize,
    ) -> Vec<WindowChoice<'_>> {
        let mut used = 0;
        let mut window = Vec::new();
        for position in self.offset..self.visible.len() {
            let height = 2;
            if !window.is_empty() && used + height > max_lines {
                break;
            }
            used += height;
            window.push(WindowChoice {
                position,
                choice: &self.choices[self.visible[position]],
            });
        }
        window
    }

    pub(in crate::cli::interactive::session_picker::picker) fn counts(&self) -> (usize, usize) {
        let now = crate::util::now_secs();
        self.choices
            .iter()
            .fold((0, 0), |(sessions, agents), choice| match choice {
                HomeChoice::Session(choice)
                    if self.range.includes(&choice.row, now)
                        && self
                            .project_filter
                            .as_deref()
                            .is_none_or(|project| choice.row.belongs_to(project)) =>
                {
                    (sessions + 1, agents)
                }
                HomeChoice::Agent(_) => (sessions, agents + 1),
                HomeChoice::Session(_) => (sessions, agents),
            })
    }
}
