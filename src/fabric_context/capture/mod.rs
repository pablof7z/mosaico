//! Canonical, now/cursor-independent capture of metadata, rosters, presence,
//! messages, and reactions. Captures are supersets: expiration, time windows,
//! and cursor selection remain pure assembly decisions.

mod activity;
mod members;
mod read;
mod topology;

pub(super) use activity::StatusCap;
pub(crate) use members::MembersInput;
pub(super) use topology::WorkspaceCap;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{missing_channel_warning, FabricContextInput};
use crate::state::Store;

/// The canonical, replayable inputs the fabric view derives from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewInputs {
    pub(crate) meta: MetaInput,
    pub(crate) members: MembersInput,
    pub(crate) presence: PresenceInput,
    pub(crate) messages: MessagesInput,
    #[serde(default)]
    pub(crate) reactions: ReactionsInput,
}

impl ViewInputs {
    /// Whether the caller forced a render (suppresses the empty-snapshot gate).
    pub(crate) fn force(&self) -> bool {
        self.meta.force
    }
}

/// Channel/subchannel metadata + per-render identity (all now/cursor-free).
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetaInput {
    pub(super) self_row: Option<SelfCap>,
    pub(super) hosts: Vec<HostCap>,
    pub(super) workspaces: Vec<WorkspaceCap>,
    pub(super) active_channels: BTreeSet<String>,
    pub(super) current_workspace: String,
    pub(super) warnings: Vec<String>,
    pub(super) self_pubkey: String,
    pub(super) self_ref: String,
    /// This daemon's host label for non-session fallback refs.
    #[serde(default)]
    pub(super) local_host: String,
    pub(super) force: bool,
}

/// Presence/status rows (superset, updated_at DESC) with the fields the render
/// keys on: state/activity/title plus last_seen/updated_at/expiration.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PresenceInput {
    pub(super) statuses: BTreeMap<String, Vec<StatusCap>>,
}

/// Chat/mentions: per-channel captured events + forced (inbox) seeds.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MessagesInput {
    pub(super) channels: BTreeMap<String, MsgBundle>,
}

/// Reactions on the caller's own recent messages (a cursor-independent superset;
/// the cursor delta is applied at assemble time).
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReactionsInput {
    pub(super) rows: Vec<super::reactions::ReactionCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SelfCap {
    pub(super) name: String,
    #[serde(default)]
    pub(super) host: String,
    #[serde(default)]
    pub(super) headless: bool,
    #[serde(default)]
    pub(super) title: String,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SummaryCap {
    pub(super) name: String,
    pub(super) channel: String,
    pub(super) about: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct AgentCap {
    pub(super) reference: String,
    pub(super) about: String,
    pub(super) created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HostCap {
    pub(super) name: String,
    pub(super) agents: Vec<AgentCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ChannelCap {
    pub(super) h: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) reference: String,
    pub(super) about: String,
    pub(super) updated_at: u64,
    pub(super) latest_message_at: Option<u64>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct MsgBundle {
    pub(super) events: Vec<EvCap>,
    pub(super) forced: Vec<EvCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct EvCap {
    pub(super) id: String,
    pub(super) channel_ref: String,
    pub(super) from_ref: String,
    pub(super) recipient_refs: Vec<String>,
    pub(super) created_at: u64,
    pub(super) body: String,
    pub(super) truncated: bool,
    /// Self-mention derived from the event's OWN `p` tags (always false for a
    /// forced seed, whose mention intent is carried by `forced_mention`).
    pub(super) mentions_self: bool,
    /// A forced (inbox) seed that was flagged as a direct mention.
    pub(super) forced_mention: bool,
}

/// Read the store once and freeze the four canonical inputs. `now`/`cursor`
/// filtering stays out of the superset captures so the reconciler owns that
/// decision.
pub(crate) fn capture_inputs(
    store: &Store,
    input: &FabricContextInput<'_>,
) -> anyhow::Result<ViewInputs> {
    // Missing relay metadata is an explicit outer-view degraded case: retain the
    // requested scope only to label the warning, never as an alternate binding.
    let current_workspace = if store.get_channel(input.scope)?.is_some() {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .root_for_channel(input.scope)?
    } else {
        input.scope.to_string()
    };
    let active_channels = read::active_channels(store, input.session)
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

    let mut refs: BTreeMap<String, String> = BTreeMap::new();
    let mut agent_slugs: BTreeMap<String, String> = BTreeMap::new();
    let mut backend: BTreeSet<String> = BTreeSet::new();
    let mut has_handle: BTreeSet<String> = BTreeSet::new();
    let mut roster: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
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
        let members: BTreeMap<String, String> = store
            .list_channel_members(h)
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.pubkey, m.role))
            .collect();
        let chan_statuses = activity::status_caps(
            store,
            h,
            input.local_host,
            &mut refs,
            &mut agent_slugs,
            &mut backend,
            &mut has_handle,
        );
        for pk in members.keys() {
            read::resolve_pubkey(
                store,
                pk,
                input.local_host,
                &mut refs,
                &mut agent_slugs,
                &mut backend,
                &mut has_handle,
            );
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
        read::resolve_pubkey(
            store,
            input.self_pubkey,
            input.local_host,
            &mut refs,
            &mut agent_slugs,
            &mut backend,
            &mut has_handle,
        );
        if let Some(session) = input.session {
            agent_slugs.insert(input.self_pubkey.to_string(), session.agent_slug.clone());
        }
    }
    // Exclude this daemon's own management key by identity, independent of whether
    // its kind:0 has been fetched into the local cache — on a cold cache (post-reset)
    // the profile is absent, so `resolve_pubkey`'s is_backend flag alone would let
    // the mgmt key leak into the roster. Assemble filters against this `backend` set.
    if !input.backend_pubkey.is_empty() {
        backend.insert(input.backend_pubkey.to_string());
    }

    let self_ref =
        crate::idref::agent_ref_from(input.self_slug, input.local_host, input.local_host);
    let meta = MetaInput {
        self_row: input.session.map(|s| read::self_cap(store, s, input)),
        hosts,
        workspaces,
        active_channels,
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
            refs,
            agent_slugs,
            backend,
            activity,
            has_handle,
        },
        presence: PresenceInput { statuses },
        messages: MessagesInput { channels: messages },
        reactions: ReactionsInput {
            rows: reaction_rows,
        },
    })
}
