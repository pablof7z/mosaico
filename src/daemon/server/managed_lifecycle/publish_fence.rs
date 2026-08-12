//! Exact session readiness authority held through one following publish.

use super::*;
use crate::domain::ChatMessage;
use crate::fabric::provider::{chat::PublishedChat, ConfirmedGroupScope};

/// Serialize one exact session route through readiness and the following
/// publish. A successful readiness repair carries only an ephemeral NMP result;
/// it never becomes a second roster projection.
#[derive(Debug)]
pub(in crate::daemon::server) struct SessionPublishFence<'a> {
    pub(super) _lane: tokio::sync::MutexGuard<'a, ()>,
    pub(super) confirmed_scope: Option<ConfirmedGroupScope>,
}

impl SessionPublishFence<'_> {
    pub(in crate::daemon::server) async fn publish_chat(
        &self,
        state: &Arc<DaemonState>,
        chat: &ChatMessage,
        keys: &nostr::Keys,
    ) -> Result<PublishedChat> {
        match self.confirmed_scope.as_ref() {
            Some(scope) => {
                state
                    .provider()
                    .publish_chat_after_confirmed_membership(chat, keys, scope)
                    .await
            }
            None => state.provider().publish_chat_checked(chat, keys).await,
        }
    }
}
