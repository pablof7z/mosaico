use super::super::HomeChoice;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProjectOption {
    pub(super) id: Option<String>,
    pub(super) name: String,
    pub(super) path: Option<String>,
}

#[derive(Debug)]
pub(super) struct ProjectPicker {
    pub(super) options: Vec<ProjectOption>,
    pub(super) visible: Vec<usize>,
    pub(super) query: String,
    pub(super) cursor: usize,
    pub(super) offset: usize,
}

impl ProjectPicker {
    pub(super) fn new(choices: &[HomeChoice], selected: Option<&str>) -> Self {
        let mut projects = BTreeMap::new();
        for workspace in choices
            .iter()
            .filter_map(|choice| match choice {
                HomeChoice::Session(choice) => Some(&choice.row),
                HomeChoice::Agent(_) => None,
            })
            .flat_map(|row| row.workspaces.iter())
        {
            projects
                .entry(workspace.id.clone())
                .or_insert_with(|| ProjectOption {
                    id: Some(workspace.id.clone()),
                    name: workspace.name.clone(),
                    path: workspace.path.clone(),
                });
        }
        let mut options = vec![ProjectOption {
            id: None,
            name: "All projects".into(),
            path: None,
        }];
        options.extend(projects.into_values());
        options[1..].sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        let visible = (0..options.len()).collect::<Vec<_>>();
        let cursor = selected
            .and_then(|id| {
                visible
                    .iter()
                    .position(|&index| options[index].id.as_deref() == Some(id))
            })
            .unwrap_or(0);
        Self {
            options,
            visible,
            query: String::new(),
            cursor,
            offset: 0,
        }
    }

    pub(super) fn refilter(&mut self) {
        let query = self.query.to_lowercase();
        self.visible = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| {
                query.is_empty()
                    || option.name.to_lowercase().contains(&query)
                    || option
                        .path
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.cursor = 0;
        self.offset = 0;
    }

    pub(super) fn move_up(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        self.cursor = if self.cursor == 0 {
            self.visible.len() - 1
        } else {
            self.cursor - 1
        };
    }

    pub(super) fn move_down(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        self.cursor = (self.cursor + 1) % self.visible.len();
    }

    pub(super) fn ensure_visible(&mut self, rows: usize) {
        if rows == 0 || self.visible.is_empty() {
            self.offset = 0;
            return;
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + rows {
            self.offset = self.cursor + 1 - rows;
        }
        self.offset = self.offset.min(self.visible.len().saturating_sub(rows));
    }

    pub(super) fn window(&self, rows: usize) -> impl Iterator<Item = (usize, &ProjectOption)> {
        let end = (self.offset + rows).min(self.visible.len());
        self.visible[self.offset..end]
            .iter()
            .enumerate()
            .map(move |(relative, &index)| (self.offset + relative, &self.options[index]))
    }
}
