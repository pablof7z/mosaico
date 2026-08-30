use anyhow::{Context, Result};
use fava_simple_groups::SimpleGroup;
use fava_simple_groups::{
    create_group, delete_group, edit_metadata, put_user, remove_user, MetadataEdit,
};
use nostr::event::{EventId, UnsignedEvent};
use nostr::key::PublicKey;

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
        author: PublicKey,
    ) -> Result<()> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        if users.is_empty() {
            return Ok(());
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        tokio::task::spawn_blocking(move || {
            let group = simple_group(&group, host.relays.iter().cloned())?;
            let first = users
                .first()
                .context("building put-user event requires at least one user")?;
            if users.iter().any(|user| user.roles != first.roles) {
                anyhow::bail!("one put-user event requires one shared role set");
            }
            let pubkeys = users.iter().map(|user| user.pubkey).collect::<Vec<_>>();
            let roles = first.roles.iter().map(String::as_str).collect::<Vec<_>>();
            let builder =
                put_user(&group, &pubkeys, &roles).context("building put-user event")?;
            publish_group_builder_and_wait(&host, builder, author, "put-user")?;
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
        author: PublicKey,
    ) -> Result<()> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        if pubkeys.is_empty() {
            return Ok(());
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        tokio::task::spawn_blocking(move || {
            let group = simple_group(&group, host.relays.iter().cloned())?;
            let builder =
                remove_user(&group, &pubkeys).context("building remove-user event")?;
            publish_group_builder_and_wait(&host, builder, author, "remove-user")?;
            Ok(())
        })
        .await
        .context("joining remove-group-users task")?
    }

    /// Publish a kind:9002 edit-metadata for `group` and await settlement.
    pub(crate) async fn publish_metadata_edit_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        edit: &MetadataEdit,
        author: PublicKey,
    ) -> Result<EventId> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        let edit = edit.clone();
        tokio::task::spawn_blocking(move || {
            let group = simple_group(&group, host.relays.iter().cloned())?;
            let builder =
                edit_metadata(&group, &edit).context("building edit-metadata event")?;
            publish_group_builder_and_wait(&host, builder, author, "edit-metadata")
        })
        .await
        .context("joining edit-metadata task")?
    }

    /// Publish a kind:9007 create-group for `group` and await settlement.
    pub(crate) async fn publish_create_group_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        author: PublicKey,
    ) -> Result<EventId> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        tokio::task::spawn_blocking(move || {
            let group = simple_group(&group, host.relays.iter().cloned())?;
            let builder = create_group(&group).context("building create-group event")?;
            publish_group_builder_and_wait(&host, builder, author, "create-group")
        })
        .await
        .context("joining create-group task")?
    }

    /// Publish a kind:9007 create-subgroup for `child` under `parent` and await settlement.
    pub(crate) async fn publish_create_subgroup_and_wait(
        self: &std::sync::Arc<Self>,
        child: &str,
        parent: &str,
        author: PublicKey,
    ) -> Result<EventId> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let host = std::sync::Arc::clone(self);
        let child = child.to_string();
        let parent = parent.to_string();
        tokio::task::spawn_blocking(move || {
            let h_tag = nostr::event::Tag::parse(["h", &child])
                .map_err(|error| anyhow::anyhow!("h tag: {error}"))?;
            let parent_tag = nostr::event::Tag::parse(["parent", &parent])
                .map_err(|error| anyhow::anyhow!("parent tag: {error}"))?;
            let unsigned = fava::EventBuilder::new(fava::Kind::from_u16(9007))
                .tags([h_tag, parent_tag])
                .by(author)
                .build()
                .map_err(anyhow::Error::from)?;
            publish_unsigned_and_wait(&host, unsigned, "create-subgroup")
        })
        .await
        .context("joining create-subgroup task")?
    }

    /// Publish a kind:9008 delete-group for `group` and await settlement.
    pub(crate) async fn publish_delete_group_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        author: PublicKey,
    ) -> Result<EventId> {
        let host = std::sync::Arc::clone(self);
        let group = group.to_string();
        tokio::task::spawn_blocking(move || {
            let group = simple_group(&group, host.relays.iter().cloned())?;
            let builder = delete_group(&group).context("building delete-group event")?;
            publish_group_builder_and_wait(&host, builder, author, "delete-group")
        })
        .await
        .context("joining delete-group task")?
    }
}

/// Fava finalizes the routed management builder, signs it with its registered
/// author signer, and owns the resulting durable delivery lifecycle.
fn publish_group_builder_and_wait(
    host: &NmpHost,
    builder: fava::EventBuilder,
    author: PublicKey,
    operation: &str,
) -> Result<EventId> {
    let write = host
        .fava
        .by(author)
        .publish(builder)
        .with_context(|| format!("publishing {operation} event"))?;
    let event_id = write
        .receipt()
        .with_context(|| format!("reading accepted {operation} receipt"))?
        .current
        .id();
    tokio::runtime::Handle::current()
        .block_on(write.settled(fava::all_terminal()))
        .with_context(|| format!("awaiting {operation} delivery"))?;
    Ok(event_id)
}

/// Fava signs the unsigned subgroup event using its registered author signer,
/// then owns the resulting durable delivery lifecycle.
fn publish_unsigned_and_wait(
    host: &NmpHost,
    mut unsigned: UnsignedEvent,
    operation: &str,
) -> Result<EventId> {
    super::scrub_unsigned(&mut unsigned);
    unsigned.ensure_id();
    let event_id = unsigned.id.context("building group-management event id")?;
    let write = host
        .fava
        .to(host.relays.iter().cloned().collect::<Vec<_>>())
        .with_context(|| format!("building relay route for {operation}"))?
        .publish(unsigned)
        .with_context(|| format!("publishing {operation} event"))?;
    tokio::runtime::Handle::current()
        .block_on(write.settled(fava::all_terminal()))
        .with_context(|| format!("awaiting {operation} delivery"))?;
    Ok(event_id)
}

/// Build a `SimpleGroup` from a group id and the configured relay URLs.
fn simple_group(
    group: &str,
    relays: impl IntoIterator<Item = fava::RelayUrl>,
) -> Result<SimpleGroup> {
    let relays: Vec<_> = relays.into_iter().collect();
    if relays.is_empty() {
        anyhow::bail!("no configured relay for NIP-29 group write");
    }
    SimpleGroup::new(group, relays)
        .map_err(|error| anyhow::anyhow!("building SimpleGroup for {group:?}: {error}"))
}
