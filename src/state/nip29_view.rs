//! Product reads borrow the complete current NIP-29 view.

use std::sync::Arc;

use crate::fabric::nip29::view::Nip29View;

use super::Store;

impl Store {
    /// The current decoded level. Accessors borrow from it; nothing here
    /// decodes or rebuilds.
    pub(super) fn view(&self) -> Arc<Nip29View> {
        #[cfg(test)]
        if let Some(view) = self
            .test_snapshots
            .read()
            .expect("test snapshots poisoned")
            .view
            .clone()
        {
            return view;
        }
        self.nip29_view
            .as_ref()
            .map(|view| view.current())
            .unwrap_or_default()
    }
}
