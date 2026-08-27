use anyhow::{Context, Result};
use fava_simple_groups::{create_group, delete_group, edit_metadata, put_user, remove_user, MetadataEdit};
use fava_simple_groups::SimpleGroup;
use nostr::event::EventId;
use nostr::key::{Keys, PublicKey};

use super::NmpHost;

/// A group user with optional roles, matching the shape callers already
/// assemble (`pubkey` + `roles`).
pub(crate) struct GroupUser {
    pub(crate) pubkey: PublicKey,
    pub(crate) roles: Vec<String>,
}

impl NmpHost {
    /// Add every named user in one kind:9000 event and await delivery to every
    /// configured group host.
    pub(crate) async fn add_group_users_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        users: Vec<GroupUser>,
        keys: &Keys,
    ) -> Result<()> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            let sg = simple_group(&group, host.relays.iter().cloned())?;
            let author = keys.public_key();
            for user in &users {
                let role_refs: Vec<&str> = user.roles.iter().map(String::as_str).collect();
                let unsigned = put_user(author, &sg, &user.pubkey, &role_refs)
                    .context("building put-user event")?;
                let write = host
                    .fava
                    .to(host.relays.iter().cloned().collect::<Vec<_>>())
                    .context("building relay route for put-user")?
                    .publish(unsigned)
                    .context("publishing put-user event")?;
                tokio::runtime::Handle::current()
                    .block_on(write.settled(fava::all_terminal()))
                    .context("awaiting put-user delivery")?;
            }
            Ok(())
        })
        .await
        .context("joining add-group-users task")?
    }

    /// Remove every named user in one kind:9001 event and await delivery.
    pub(crate) async fn remove_group_users_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        pubkeys: Vec<PublicKey>,
        keys: &Keys,
    ) -> Result<()> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            let sg = simple_group(&group, host.relays.iter().cloned())?;
            let author = keys.public_key();
            for pubkey in &pubkeys {
                let unsigned = remove_user(author, &sg, pubkey)
                    .context("building remove-user event")?;
                let write = host
                    .fava
                    .to(host.relays.iter().cloned().collect::<Vec<_>>())
                    .context("building relay route for remove-user")?
                    .publish(unsigned)
                    .context("publishing remove-user event")?;
                tokio::runtime::Handle::current()
                    .block_on(write.settled(fava::all_terminal()))
                    .context("awaiting remove-user delivery")?;
            }
            Ok(())
        })
        .await
        .context("joining remove-group-users task")?
    }
}

impl NmpHost {
    /// Publish a kind:9002 edit-metadata for `group` and await settlement.
    pub(crate) async fn publish_metadata_edit_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        edit: &MetadataEdit,
        keys: &Keys,
    ) -> Result<EventId> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        let edit = edit.clone();
        tokio::task::spawn_blocking(move || {
            let sg = simple_group(&group, host.relays.iter().cloned())?;
            let mut unsigned = edit_metadata(keys.public_key(), &sg, &edit)
                .context("building edit-metadata event")?;
            unsigned.ensure_id();
            let event_id = unsigned.id.expect("ensure_id sets id");
            let write = host
                .fava
                .to(host.relays.iter().cloned().collect::<Vec<_>>())
                .context("building relay route for edit-metadata")?
                .publish(unsigned)
                .context("publishing edit-metadata event")?;
            tokio::runtime::Handle::current()
                .block_on(write.settled(fava::all_terminal()))
                .context("awaiting edit-metadata settlement")?;
            Ok(event_id)
        })
        .await
        .context("joining edit-metadata task")?
    }

    /// Publish a kind:9007 create-group for `group` (with optional parent) and await settlement.
    pub(crate) async fn publish_create_group_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        keys: &Keys,
    ) -> Result<EventId> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            let sg = simple_group(&group, host.relays.iter().cloned())?;
            let mut unsigned = create_group(keys.public_key(), &sg)
                .context("building create-group event")?;
            unsigned.ensure_id();
            let event_id = unsigned.id.expect("ensure_id sets id");
            let write = host
                .fava
                .to(host.relays.iter().cloned().collect::<Vec<_>>())
                .context("building relay route for create-group")?
                .publish(unsigned)
                .context("publishing create-group event")?;
            tokio::runtime::Handle::current()
                .block_on(write.settled(fava::all_terminal()))
                .context("awaiting create-group settlement")?;
            Ok(event_id)
        })
        .await
        .context("joining create-group task")?
    }

    /// Publish a kind:9007 create-subgroup for `child` under `parent` and await settlement.
    pub(crate) async fn publish_create_subgroup_and_wait(
        self: &std::sync::Arc<Self>,
        child: &str,
        parent: &str,
        keys: &Keys,
    ) -> Result<EventId> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let child = child.to_string();
        let parent = parent.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            // kind:9007 with h=child, parent=parent_id
            use nostr::event::{FinalizeUnsignedEvent, Kind, Tag};
            use nostr::event::EventBuilder as NostrBuilder;
            let h_tag = Tag::parse(["h", &child])
                .map_err(|e| anyhow::anyhow!("h tag: {e}"))?;
            let parent_tag = Tag::parse(["parent", &parent])
                .map_err(|e| anyhow::anyhow!("parent tag: {e}"))?;
            let mut unsigned = NostrBuilder::new(Kind::from_u16(9007), "")
                .tags([h_tag, parent_tag])
                .finalize_unsigned(keys.public_key());
            unsigned.ensure_id();
            let event_id = unsigned.id.expect("ensure_id sets id");
            let write = host
                .fava
                .to(host.relays.iter().cloned().collect::<Vec<_>>())
                .context("building relay route for create-subgroup")?
                .publish(unsigned)
                .context("publishing create-subgroup event")?;
            tokio::runtime::Handle::current()
                .block_on(write.settled(fava::all_terminal()))
                .context("awaiting create-subgroup settlement")?;
            Ok(event_id)
        })
        .await
        .context("joining create-subgroup task")?
    }

    /// Publish a kind:9008 delete-group for `group` and await settlement.
    pub(crate) async fn publish_delete_group_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        keys: &Keys,
    ) -> Result<EventId> {
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            let sg = simple_group(&group, host.relays.iter().cloned())?;
            let mut unsigned = delete_group(keys.public_key(), &sg)
                .context("building delete-group event")?;
            unsigned.ensure_id();
            let event_id = unsigned.id.expect("ensure_id sets id");
            let write = host
                .fava
                .to(host.relays.iter().cloned().collect::<Vec<_>>())
                .context("building relay route for delete-group")?
                .publish(unsigned)
                .context("publishing delete-group event")?;
            tokio::runtime::Handle::current()
                .block_on(write.settled(fava::all_terminal()))
                .context("awaiting delete-group settlement")?;
            Ok(event_id)
        })
        .await
        .context("joining delete-group task")?
    }
}

/// Build a `SimpleGroup` from a group id and the configured relay URLs.
fn simple_group(
    group: &str,
    relays: impl IntoIterator<Item = fava::RelayUrl>,
) -> Result<SimpleGroup> {
    let relay_vec: Vec<_> = relays.into_iter().collect();
    if relay_vec.is_empty() {
        anyhow::bail!("no configured relay for NIP-29 group write");
    }
    SimpleGroup::new(group, relay_vec)
        .map_err(|e| anyhow::anyhow!("building SimpleGroup for {group:?}: {e}"))
}
