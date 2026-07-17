use super::{Channel, Store};
use anyhow::Result;

/// Read-only capability for callers that assemble views from the store.
#[derive(Clone, Copy)]
pub(crate) struct StoreReader<'a> {
    store: &'a Store,
}

impl Store {
    pub(crate) fn reader(&self) -> StoreReader<'_> {
        StoreReader { store: self }
    }
}

impl StoreReader<'_> {
    pub(crate) fn list_channels(self) -> Result<Vec<Channel>> {
        self.store.list_channels()
    }

    pub(crate) fn root_channel_of(self, channel_h: &str) -> Result<Option<String>> {
        self.store.root_channel_of(channel_h)
    }

    pub(crate) fn workspace_path(self, channel_h: &str) -> Result<Option<String>> {
        self.store.workspace_path(channel_h)
    }
}
