//! Repairing a workspace root whose relay-signed name drifted off its group id.
//!
//! Driven by the retained group-records observation and by nothing else. The
//! roots this backend may rename are selected directly from the current NMP
//! snapshots whose relay-signed admin list names the management key. There is
//! no second group-record projection to query afterwards.
//!
//! A root is considered only when THIS delivery carries relay metadata for it.
//! A local `channel_init` reservation is therefore never mistaken for a name
//! the relay actually signed and never causes a repair publication.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::DaemonState;

/// Rename every delivered root whose relay-signed name is not its group id.
///
/// `attempted` bounds this to one publish per root for the life of the
/// observation. An accepted rename needs no retry — the relay's next record
/// carries the corrected name and stops matching — and a rejected one must not
/// be re-published on every subsequent delivery.
///
/// The publishes are spawned rather than awaited: this runs on the retained
/// observation drain, and a slow relay write must not hold up the next snapshot.
/// This function only borrows NMP's current delivery.
pub(super) fn repair_delivered(
    state: &Arc<DaemonState>,
    snapshots: &[nmp::nip29::GroupSnapshot],
    attempted: &mut BTreeSet<String>,
) {
    let Some(backend_pubkey) = state.backend_pubkey() else {
        return;
    };
    let bindings = match state.with_store(|store| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).bindings()
    }) {
        Ok(bindings) => bindings
            .into_iter()
            .map(|binding| binding.channel_h)
            .collect::<BTreeSet<_>>(),
        Err(error) => {
            tracing::error!(%error, "workspace root repair binding lookup failed");
            return;
        }
    };
    let roots = roots_needing_workspace_name(snapshots, &backend_pubkey, &bindings, attempted);
    if roots.is_empty() {
        return;
    }
    attempted.extend(roots.iter().cloned());
    let state = state.clone();
    tokio::spawn(async move {
        for root in roots {
            if !state.provider().nip29_set_group_name(&root, &root).await {
                tracing::warn!(
                    channel = %root,
                    "workspace root name repair was rejected"
                );
            }
        }
    });
}

/// The delivered roots this backend both manages and must rename.
///
/// "Manages" is either the relay-signed admin grant or a local workspace
/// binding: a root bound to a directory on this machine is this backend's to
/// keep named even while the admin record is in flight.
fn roots_needing_workspace_name(
    snapshots: &[nmp::nip29::GroupSnapshot],
    backend_pubkey: &str,
    bound_roots: &BTreeSet<String>,
    attempted: &BTreeSet<String>,
) -> Vec<String> {
    snapshots
        .iter()
        .filter(|snapshot| !attempted.contains(&snapshot.id))
        .filter(|snapshot| super::is_root(snapshot))
        .filter(|snapshot| {
            snapshot
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.name.as_deref())
                .is_none_or(|name| name.trim() != snapshot.id)
        })
        .filter(|snapshot| {
            bound_roots.contains(&snapshot.id)
                || snapshot
                    .admins
                    .iter()
                    .any(|admin| admin.pubkey.to_hex() == backend_pubkey)
        })
        .map(|snapshot| snapshot.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use nmp::nip29::{GroupAvailability, GroupMetadata, GroupSnapshot, ListedSubject};
    use nmp::RelayUrl;
    use nostr::{EventId, Keys, Timestamp};

    fn snapshot(
        id: &str,
        name: &str,
        parent: Option<&str>,
        admin: Option<nostr::PublicKey>,
    ) -> GroupSnapshot {
        let host = RelayUrl::parse("wss://relay.example").unwrap();
        let mut tags = Vec::new();
        if let Some(parent) = parent {
            tags.push(vec!["parent".to_string(), parent.to_string()]);
        }
        GroupSnapshot {
            id: id.to_string(),
            metadata: Some(GroupMetadata {
                name: Some(name.to_string()),
                about: None,
                picture: None,
                tags,
                as_of: Timestamp::from(1),
                event_id: EventId::all_zeros(),
                host: host.clone(),
            }),
            admins: admin
                .map(|pubkey| ListedSubject {
                    pubkey,
                    role: Some("admin".to_string()),
                    hosts: BTreeSet::from([host]),
                })
                .into_iter()
                .collect(),
            members: Vec::new(),
            availability: GroupAvailability::Ready,
            per_host: BTreeMap::new(),
            disagreements: BTreeSet::new(),
        }
    }

    #[test]
    fn only_managed_misnamed_roots_need_repair() {
        let backend = Keys::generate().public_key();
        let remote = Keys::generate().public_key();
        let snapshots = vec![
            snapshot("one", "wrong", None, Some(backend)),
            snapshot("two", "two", None, Some(backend)),
            snapshot("remote", "wrong", None, Some(remote)),
            snapshot("bound", "also-wrong", None, None),
            snapshot("child", "wrong", Some("one"), Some(backend)),
        ];
        assert_eq!(
            roots_needing_workspace_name(
                &snapshots,
                &backend.to_hex(),
                &BTreeSet::from(["bound".to_string()]),
                &BTreeSet::new(),
            ),
            vec!["bound", "one"]
        );
    }

    #[test]
    fn an_attempted_root_is_not_republished() {
        let backend = Keys::generate().public_key();
        assert!(roots_needing_workspace_name(
            &[snapshot("one", "wrong", None, Some(backend))],
            &backend.to_hex(),
            &BTreeSet::new(),
            &BTreeSet::from(["one".to_string()]),
        )
        .is_empty());
    }
}
