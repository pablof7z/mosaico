use super::data::ChannelNode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub(super) struct FlatRow {
    pub path: String,
    pub about: String,
    pub agents: Option<usize>,
    pub last_activity: Option<String>,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
    pub child_count: usize,
}

#[derive(Debug)]
pub(super) enum Pending {
    ConfirmDelete { path: String },
}

#[derive(Debug)]
pub(super) enum Exit {
    Quit,
    Edit { path: String, about: String },
    Delete { path: String },
}

#[derive(Debug)]
pub(super) struct PickerState {
    forest: Vec<ChannelNode>,
    expanded: BTreeSet<String>,
    rows: Vec<FlatRow>,
    visible: Vec<usize>,
    query: String,
    cursor: usize,
    offset: usize,
    pending: Option<Pending>,
    notice: Option<String>,
}

impl PickerState {
    pub(super) fn new(forest: Vec<ChannelNode>) -> Self {
        let mut expanded = BTreeSet::new();
        expand_all(&forest, &mut expanded);
        let mut state = Self {
            forest,
            expanded,
            rows: Vec::new(),
            visible: Vec::new(),
            query: String::new(),
            cursor: 0,
            offset: 0,
            pending: None,
            notice: None,
        };
        state.rebuild();
        state
    }

    pub(super) fn focus_path(&mut self, path: &str) {
        if let Some(pos) = self.visible.iter().position(|&i| self.rows[i].path == path) {
            self.cursor = pos;
        }
    }

    pub(super) fn replace_forest(&mut self, forest: Vec<ChannelNode>) {
        let focus = self.current().map(|r| r.path.clone());
        let mut keep = BTreeSet::new();
        for path in self.expanded.iter() {
            if forest_contains(&forest, path) {
                keep.insert(path.clone());
            }
        }
        if keep.is_empty() {
            expand_all(&forest, &mut keep);
        }
        self.forest = forest;
        self.expanded = keep;
        self.rebuild();
        if let Some(path) = focus {
            if let Some(pos) = self.visible.iter().position(|&i| self.rows[i].path == path) {
                self.cursor = pos;
            }
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, lines: usize) -> Option<Exit> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if let Some(pending) = self.pending.take() {
            return self.handle_pending(pending, key);
        }
        self.notice = None;
        match key.code {
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.rebuild();
            }
            KeyCode::Esc | KeyCode::Char('q') => return Some(Exit::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(Exit::Quit);
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-(lines as isize).max(1)),
            KeyCode::PageDown => self.move_by((lines as isize).max(1)),
            KeyCode::Home | KeyCode::Char('g') => self.cursor = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = self.visible.len().saturating_sub(1);
            }
            KeyCode::Left | KeyCode::Char('h') => self.collapse_or_parent(),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => self.expand_current(),
            KeyCode::Char('e' | 'E') => {
                if let Some(row) = self.current() {
                    return Some(Exit::Edit {
                        path: row.path.clone(),
                        about: row.about.clone(),
                    });
                }
            }
            KeyCode::Char('d' | 'D') => {
                if let Some(row) = self.current() {
                    if row.depth == 0 && !row.path.contains('/') {
                        // Public root paths are `#name` (no slash).
                        self.notice = Some(format!(
                            "{} is a workspace root — delete children or archive instead",
                            row.path
                        ));
                    } else if row.has_children {
                        self.notice = Some(format!(
                            "{} has {} child channel(s) — delete children first",
                            row.path, row.child_count
                        ));
                    } else {
                        self.pending = Some(Pending::ConfirmDelete {
                            path: row.path.clone(),
                        });
                    }
                }
            }
            KeyCode::Char('r' | 'R') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // handled by picker loop as refresh
            }
            KeyCode::Backspace if !self.query.is_empty() => {
                self.query.pop();
                self.rebuild();
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(c);
                self.rebuild();
            }
            _ => {}
        }
        self.ensure_visible(lines);
        None
    }

    fn handle_pending(&mut self, pending: Pending, key: KeyEvent) -> Option<Exit> {
        match pending {
            Pending::ConfirmDelete { path } => match key.code {
                KeyCode::Char('y' | 'Y' | 'd' | 'D') => Some(Exit::Delete { path }),
                KeyCode::Esc | KeyCode::Char('n' | 'N') => None,
                _ => {
                    self.pending = Some(Pending::ConfirmDelete { path });
                    None
                }
            },
        }
    }

    pub(super) fn wants_refresh(key: &KeyEvent) -> bool {
        matches!(key.code, KeyCode::Char('r' | 'R'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
    }

    pub(super) fn current(&self) -> Option<&FlatRow> {
        self.visible.get(self.cursor).map(|&i| &self.rows[i])
    }

    pub(super) fn cursor_index(&self) -> usize {
        self.cursor
    }

    pub(super) fn window(&self, height: usize) -> impl Iterator<Item = (usize, &FlatRow)> {
        let end = (self.offset + height).min(self.visible.len());
        self.visible[self.offset..end]
            .iter()
            .enumerate()
            .map(move |(pos, &idx)| (self.offset + pos, &self.rows[idx]))
    }

    pub(super) fn ensure_visible(&mut self, lines: usize) {
        if self.visible.is_empty() {
            self.cursor = 0;
            self.offset = 0;
            return;
        }
        self.cursor = self.cursor.min(self.visible.len() - 1);
        let lines = lines.max(1);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + lines {
            self.offset = self.cursor + 1 - lines;
        }
    }

    pub(super) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(super) fn set_notice(&mut self, notice: impl Into<String>) {
        self.notice = Some(notice.into());
    }

    pub(super) fn pending(&self) -> Option<&Pending> {
        self.pending.as_ref()
    }

    pub(super) fn query(&self) -> &str {
        &self.query
    }

    pub(super) fn position_label(&self) -> String {
        if self.visible.is_empty() {
            "0/0".into()
        } else {
            format!("{}/{}", self.cursor + 1, self.visible.len())
        }
    }

    pub(super) fn row_count(&self) -> usize {
        self.visible.len()
    }

    fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, self.visible.len() as isize - 1) as usize;
    }

    fn expand_current(&mut self) {
        let Some(row) = self.current() else {
            return;
        };
        if !row.has_children {
            return;
        }
        let path = row.path.clone();
        if self.expanded.insert(path) {
            self.rebuild();
        }
    }

    fn collapse_or_parent(&mut self) {
        let Some(row) = self.current().cloned() else {
            return;
        };
        if row.has_children && self.expanded.contains(&row.path) {
            self.expanded.remove(&row.path);
            self.rebuild();
            return;
        }
        if row.depth == 0 {
            return;
        }
        // Jump to parent path prefix.
        let parent = parent_path(&row.path);
        if let Some(pos) = self
            .visible
            .iter()
            .position(|&i| self.rows[i].path == parent)
        {
            self.cursor = pos;
        }
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        flatten(&self.forest, 0, &self.expanded, &mut self.rows);
        let q = self.query.to_ascii_lowercase();
        self.visible = if q.is_empty() {
            (0..self.rows.len()).collect()
        } else {
            self.rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    row.path.to_ascii_lowercase().contains(&q)
                        || row.about.to_ascii_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect()
        };
        if self.cursor >= self.visible.len() {
            self.cursor = self.visible.len().saturating_sub(1);
        }
    }
}

fn expand_all(nodes: &[ChannelNode], expanded: &mut BTreeSet<String>) {
    for node in nodes {
        if !node.children.is_empty() {
            expanded.insert(node.path.clone());
            expand_all(&node.children, expanded);
        }
    }
}

fn forest_contains(nodes: &[ChannelNode], path: &str) -> bool {
    nodes
        .iter()
        .any(|n| n.path == path || forest_contains(&n.children, path))
}

fn flatten(
    nodes: &[ChannelNode],
    depth: usize,
    expanded: &BTreeSet<String>,
    out: &mut Vec<FlatRow>,
) {
    for node in nodes {
        let has_children = !node.children.is_empty();
        let is_expanded = has_children && expanded.contains(&node.path);
        out.push(FlatRow {
            path: node.path.clone(),
            about: node.about.clone(),
            agents: node.agents,
            last_activity: node.last_activity.clone(),
            depth,
            has_children,
            expanded: is_expanded,
            child_count: node.children.len(),
        });
        if is_expanded {
            flatten(&node.children, depth + 1, expanded, out);
        }
    }
}

fn parent_path(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => path.to_string(),
    }
}

#[cfg(test)]
#[path = "state/tests.rs"]
mod tests;
