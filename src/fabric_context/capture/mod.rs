//! Canonical, now/cursor-independent capture of metadata, rosters, presence,
//! messages, and reactions. Captures are supersets: expiration, time windows,
//! and cursor selection remain pure assembly decisions.

mod activity;
mod members;
mod model;
mod read;
mod topology;

pub(super) use activity::StatusCap;
pub(crate) use members::MembersInput;
pub(crate) use model::*;
pub(super) use topology::WorkspaceCap;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{missing_channel_warning, FabricContextInput};
use crate::state::Store;

/// Read the store once and freeze the four canonical inputs. `now`/`cursor`
/// filtering stays out of the superset captures so the reconciler owns that
/// decision.
pub(crate) fn capture_inputs(
    store: &Store,
    input: &FabricContextInput<'_>,
) -> anyhow::Result<ViewInputs> {
    // Missing relay metadata is an explicit outer-view degraded case: retain the
    // requested scope only to label the warning, never as an alternate binding.
    // An ancestry that does not resolve is the same degraded case as absent
    // metadata: the parent's kind:39000 has not arrived, so the scope is the
    // best root we can honestly name yet.
    let current_workspace = store
        .get_channel(input.scope)?
        .and_then(|_| {
            crate::daemon::workspace_path::WorkspacePathResolver::new(store)
                .root_for_channel(input.scope)
                .ok()
        })
        .unwrap_or_else(|| input.scope.to_string());
    let joined_channels = read::joined_channels(store, input.session)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let selected_channels = read::selected_channels(store, input)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let (hosts, workspaces) = topology::capture(store)?;
    let mut warnings = input.warnings.to_vec();
    warnings.extend(
        read::missing_channels(store, input)
            .into_iter()
            .map(|channel| missing_channel_warning(&channel)),
    );

    let mut identities = read::IdentityCaps::default();
    let mut roster: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut hydrated: BTreeSet<String> = BTreeSet::new();
    let mut hosts_by_pubkey: BTreeMap<String, String> = BTreeMap::new();
    let mut activity: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut statuses: BTreeMap<String, Vec<StatusCap>> = BTreeMap::new();
    let mut messages: BTreeMap<String, MsgBundle> = BTreeMap::new();
    let forced_by_channel = read::group_forced(input.forced_messages, input.scope);

    for h in workspaces
        .iter()
        .flat_map(|workspace| &workspace.channels)
        .map(|channel| &channel.h)
    {
        // Keep relay roles in the frozen input; rendered rows do not expose them.
        let members: BTreeMap<String, String> = match store.list_channel_members(h) {
            Ok(rows) => {
                match store.has_channel_membership_snapshot(h) {
                    Ok(true) => {
                        hydrated.insert(h.clone());
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::debug!(channel = %h, %error, "membership hydration unavailable");
                    }
                }
                rows.into_iter()
                    .map(|member| (member.pubkey, member.role))
                    .collect()
            }
            Err(error) => {
                tracing::debug!(channel = %h, %error, "membership snapshot unavailable");
                BTreeMap::new()
            }
        };
        let chan_statuses = activity::status_caps(store, h, input.local_host, &mut identities);
        for pk in members.keys() {
            read::resolve_pubkey(store, pk, input.local_host, &mut identities);
            hosts_by_pubkey
                .entry(pk.clone())
                .or_insert_with(|| read::profile_host(store, pk));
        }
        activity.insert(
            h.clone(),
            store
                .latest_message_at_by_pubkey(h)
                .unwrap_or_default()
                .into_iter()
                .collect(),
        );
        roster.insert(h.clone(), members);
        statuses.insert(h.clone(), chan_statuses);

        if selected_channels.contains(h) {
            let forced = forced_by_channel.get(h).cloned().unwrap_or_default();
            messages.insert(h.clone(), read::capture_messages(store, input, h, &forced));
        }
    }
    if !input.self_pubkey.is_empty() {
        read::resolve_pubkey(store, input.self_pubkey, input.local_host, &mut identities);
        if let Some(session) = input.session {
            identities
                .agent_slugs
                .insert(input.self_pubkey.to_string(), session.agent_slug.clone());
        }
    }
    // Exclude this daemon's own management key by identity, independent of whether
    // its kind:0 has been fetched into the local cache — on a cold cache (post-reset)
    // the profile is absent, so `resolve_pubkey`'s is_backend flag alone would let
    // the mgmt key leak into the roster. Assemble filters against this `backend` set.
    if !input.backend_pubkey.is_empty() {
        identities.backend.insert(input.backend_pubkey.to_string());
    }

    let self_ref =
        crate::idref::agent_ref_from(input.self_slug, input.local_host, input.local_host);
    let meta = MetaInput {
        self_row: input.session.map(|s| read::self_cap(store, s, input)),
        hosts,
        workspaces,
        joined_channels,
        current_workspace,
        warnings,
        self_pubkey: input.self_pubkey.to_string(),
        self_ref,
        local_host: input.local_host.to_string(),
        force: input.force,
    };

    // Reactions on the caller's OWN recent messages. Floored at session creation
    // (a session-stable value, not the cursor) so the frozen input is
    // cursor-independent; assemble applies the real `> cursor` delta.
    let reaction_floor = input.session.map(|s| s.created_at).unwrap_or(0);
    let reaction_rows = super::reactions::capture_reaction_sources(
        store,
        input.self_pubkey,
        reaction_floor,
        input.local_host,
        input.backend_pubkey,
    );

    Ok(ViewInputs {
        meta,
        members: MembersInput {
            roster,
            hydrated,
            refs: identities.refs,
            agent_slugs: identities.agent_slugs,
            hosts: hosts_by_pubkey,
            backend: identities.backend,
            activity,
            has_handle: identities.has_handle,
            known_profiles: identities.known_profiles,
        },
        presence: PresenceInput { statuses },
        messages: MessagesInput { channels: messages },
        reactions: ReactionsInput {
            rows: reaction_rows,
        },
    })
}
