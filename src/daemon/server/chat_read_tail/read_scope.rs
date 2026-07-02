use crate::state::{RelayEvent, Store};

#[derive(Clone, Debug)]
pub(in crate::daemon::server) struct ChatCursor {
    pub(in crate::daemon::server) created_at: u64,
    pub(in crate::daemon::server) id: String,
}

impl ChatCursor {
    pub(in crate::daemon::server) fn new(created_at: u64) -> Self {
        Self {
            created_at,
            id: String::new(),
        }
    }

    pub(in crate::daemon::server) fn observe(&mut self, row: &RelayEvent) {
        if row.created_at > self.created_at
            || (row.created_at == self.created_at && row.id.as_str() > self.id.as_str())
        {
            self.created_at = row.created_at;
            self.id = row.id.clone();
        }
    }
}

pub(in crate::daemon::server) fn chat_read_scopes_for_store(
    store: &Store,
    scope: &str,
) -> Vec<String> {
    let mut scopes = vec![scope.to_string()];
    if store.is_root_channel(scope).unwrap_or(false) {
        use std::collections::BTreeMap;
        let mut by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for channel in store.list_channels().unwrap_or_default() {
            by_parent
                .entry(channel.parent)
                .or_default()
                .push(channel.channel_h);
        }
        let mut stack = vec![scope.to_string()];
        let mut guard = 0usize;
        while let Some(parent) = stack.pop() {
            guard += 1;
            if guard > 10_000 {
                break;
            }
            let Some(children) = by_parent.get(&parent) else {
                continue;
            };
            for child in children {
                scopes.push(child.clone());
                stack.push(child.clone());
            }
        }
    }
    scopes.sort();
    scopes.dedup();
    scopes
}
