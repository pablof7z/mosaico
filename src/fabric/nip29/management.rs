//! Product-scoped NIP-29 management event construction.

use anyhow::Result;
use nostr::prelude::{EventBuilder, Kind, PublicKey, Tag};

use super::wire::{
    KIND_GROUP_CREATE, KIND_GROUP_DELETE, KIND_GROUP_EDIT_METADATA, KIND_GROUP_PUT_USER,
    KIND_GROUP_REMOVE_USER,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetadataEdit {
    pub name: Option<String>,
    pub picture: Option<String>,
    pub about: Option<String>,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GroupUser {
    pub(crate) pubkey: PublicKey,
    pub(crate) roles: Vec<String>,
}

pub(crate) fn create_group(group: &str, parent: Option<&str>) -> Result<EventBuilder> {
    let mut builder = management(group, KIND_GROUP_CREATE)?;
    if let Some(parent) = parent.filter(|parent| !parent.is_empty()) {
        builder = builder.tag(Tag::custom("parent", [parent]));
    }
    Ok(builder)
}

pub(crate) fn delete_group(group: &str) -> Result<EventBuilder> {
    management(group, KIND_GROUP_DELETE)
}

pub(crate) fn edit_metadata(group: &str, edit: &MetadataEdit) -> Result<EventBuilder> {
    let mut builder = management(group, KIND_GROUP_EDIT_METADATA)?;
    for (name, value) in [
        ("name", edit.name.as_deref()),
        ("picture", edit.picture.as_deref()),
        ("about", edit.about.as_deref()),
    ] {
        if let Some(value) = value {
            builder = builder.tag(Tag::custom(name, [value]));
        }
    }
    if edit.closed {
        builder = builder.tag(Tag::custom("closed", Vec::<String>::new()));
    }
    Ok(builder)
}

pub(crate) fn put_users(group: &str, users: &[GroupUser]) -> Result<EventBuilder> {
    let mut builder = management(group, KIND_GROUP_PUT_USER)?;
    for user in users {
        let mut parts = vec![user.pubkey.to_hex()];
        parts.extend(user.roles.iter().cloned());
        builder = builder.tag(Tag::custom("p", parts));
    }
    Ok(builder)
}

pub(crate) fn remove_users(group: &str, users: &[PublicKey]) -> Result<EventBuilder> {
    let mut builder = management(group, KIND_GROUP_REMOVE_USER)?;
    for user in users {
        builder = builder.tag(Tag::custom("p", [user.to_hex()]));
    }
    Ok(builder)
}

fn management(group: &str, kind: u16) -> Result<EventBuilder> {
    if group.is_empty() {
        anyhow::bail!("empty NIP-29 group id");
    }
    Ok(EventBuilder::new(Kind::Custom(kind), "").tag(Tag::custom("h", [group])))
}

#[cfg(test)]
#[path = "management/tests.rs"]
mod tests;
