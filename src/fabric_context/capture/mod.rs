//! Canonical, now/cursor-INDEPENDENT capture of everything `build_view` reads
//! from the store, partitioned into the four sources the fabric snapshot derives
//! from: channel/subchannel metadata, the member roster, presence/status rows,
//! and chat/mentions. This is the pure-data boundary the Trellis reconciler feeds
//! as graph inputs (see [`crate::reconcile::hook_context`]); the wall-clock `now`
//! and the seen `cursor` are modelled as SEPARATE inputs and applied by
//! [`super::assemble::assemble_view`], never baked in here.
//!
//! Captures are SUPERSETS: every status is kept regardless of NIP-40 expiration
//! and every chat row since time 0 is kept, so the `expiration >= now` liveness
//! window and the `created_at > since` chat window remain pure functions of the
//! `now`/`cursor` inputs at assemble time rather than ambient reads. The leaf
//! store readers live in [`read`].

mod read;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{missing_channel_warning, FabricContextInput};
use crate::state::Store;

/// The four canonical, replayable inputs the fabric view derives from. Each
/// field is a distinct Trellis input node in the reconciler, so `why_changed`
/// attributes a snapshot change to exactly the source that moved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewInputs {
    pub(crate) meta: MetaInput,
    pub(crate) members: MembersInput,
    pub(crate) presence: PresenceInput,
    pub(crate) messages: MessagesInput,
}

impl ViewInputs {
    /// Reassemble from the four canonical inputs (as read back from graph nodes).
    pub(crate) fn from_parts(
        meta: MetaInput,
        members: MembersInput,
        presence: PresenceInput,
        messages: MessagesInput,
    ) -> Self {
        Self {
            meta,
            members,
            presence,
            messages,
        }
    }

    /// Whether the caller forced a render (suppresses the empty-snapshot gate).
    pub(crate) fn force(&self) -> bool {
        self.meta.force
    }
}

/// Channel/subchannel metadata + per-render identity (all now/cursor-free).
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetaInput {
    pub(super) self_row: Option<SelfCap>,
    pub(super) project: SummaryCap,
    pub(super) agents: Vec<AgentCap>,
    pub(super) channels: Vec<ChannelCap>,
    pub(super) unjoined: Vec<UnjoinedCap>,
    pub(super) warnings: Vec<String>,
    pub(super) self_pubkey: String,
    pub(super) self_ref: String,
    pub(super) force: bool,
}

/// The member roster union source: per-channel roster pubkeys, the resolved
/// display ref for every pubkey that can appear, and the backend-pubkey set.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MembersInput {
    pub(super) roster: BTreeMap<String, BTreeSet<String>>,
    pub(super) refs: BTreeMap<String, String>,
    pub(super) backend: BTreeSet<String>,
}

/// Presence/status rows (superset, updated_at DESC) with the fields the render
/// keys on: busy/activity/title plus last_seen/updated_at/expiration.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PresenceInput {
    pub(super) statuses: BTreeMap<String, Vec<StatusCap>>,
}

/// Chat/mentions: per-channel captured events + forced (inbox) seeds.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MessagesInput {
    pub(super) channels: BTreeMap<String, MsgBundle>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SelfCap {
    pub(super) agent: String,
    pub(super) backend: String,
    pub(super) session_id: String,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SummaryCap {
    pub(super) name: String,
    pub(super) about: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct AgentCap {
    pub(super) reference: String,
    pub(super) about: String,
    pub(super) created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ChannelCap {
    pub(super) h: String,
    pub(super) name: String,
    pub(super) about: String,
    pub(super) subchannels: Vec<SummaryCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct UnjoinedCap {
    pub(super) name: String,
    pub(super) about: String,
    pub(super) updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StatusCap {
    pub(super) pubkey: String,
    pub(super) busy: bool,
    pub(super) activity: String,
    pub(super) title: String,
    pub(super) last_seen: u64,
    pub(super) updated_at: u64,
    pub(super) expiration: u64,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct MsgBundle {
    pub(super) events: Vec<EvCap>,
    pub(super) forced: Vec<EvCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct EvCap {
    pub(super) id: String,
    pub(super) channel_display: String,
    pub(super) from_ref: String,
    pub(super) created_at: u64,
    pub(super) body: String,
    pub(super) truncated: bool,
    /// Self-mention derived from the event's OWN `p` tags (always false for a
    /// forced seed, whose mention intent is carried by `forced_mention`).
    pub(super) mentions_self: bool,
    /// A forced (inbox) seed that was flagged as a direct mention.
    pub(super) forced_mention: bool,
}

/// Read the store ONCE and freeze the four canonical inputs. Mirrors the store
/// reads `build_view`/`people`/`messages` perform, but keeps the `now`/`cursor`
/// filtering out (superset captures) so the reconciler owns that decision.
pub(crate) fn capture_inputs(store: &Store, input: &FabricContextInput<'_>) -> ViewInputs {
    let root = read::project_root(store, input.scope);
    let channel_hs = read::selected_channels(store, input);
    let mut warnings = input.warnings.to_vec();
    warnings.extend(
        read::missing_channels(store, input)
            .into_iter()
            .map(|channel| missing_channel_warning(&channel)),
    );

    let mut refs: BTreeMap<String, String> = BTreeMap::new();
    let mut backend: BTreeSet<String> = BTreeSet::new();
    let mut roster: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut statuses: BTreeMap<String, Vec<StatusCap>> = BTreeMap::new();
    let mut messages: BTreeMap<String, MsgBundle> = BTreeMap::new();
    let forced_by_channel = read::group_forced(input.forced_messages, input.scope);

    let mut channels = Vec::new();
    for h in &channel_hs {
        let summary = read::channel_summary(store, h);
        channels.push(ChannelCap {
            h: h.clone(),
            name: summary.name,
            about: summary.about,
            subchannels: read::subchannel_caps(store, h),
        });

        // Roster + status pubkeys → resolve refs and backend flags once.
        let members: BTreeSet<String> = store
            .list_channel_members(h)
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.pubkey)
            .collect();
        let chan_statuses: Vec<StatusCap> = store
            .live_status_for_channel(h, 0)
            .unwrap_or_default()
            .into_iter()
            .map(|s| StatusCap {
                pubkey: s.pubkey,
                busy: s.busy,
                activity: s.activity,
                title: s.title,
                last_seen: s.last_seen,
                updated_at: s.updated_at,
                expiration: s.expiration,
            })
            .collect();
        for pk in members
            .iter()
            .chain(chan_statuses.iter().map(|s| &s.pubkey))
        {
            read::resolve_pubkey(store, pk, input.local_host, &mut refs, &mut backend);
        }
        roster.insert(h.clone(), members);
        statuses.insert(h.clone(), chan_statuses);

        let forced = forced_by_channel.get(h).cloned().unwrap_or_default();
        messages.insert(h.clone(), read::capture_messages(store, input, h, &forced));
    }
    if !input.self_pubkey.is_empty() {
        read::resolve_pubkey(
            store,
            input.self_pubkey,
            input.local_host,
            &mut refs,
            &mut backend,
        );
    }

    let self_ref =
        crate::idref::agent_ref_from(input.self_slug, input.local_host, input.local_host);
    let meta = MetaInput {
        self_row: input.session.map(|s| read::self_cap(s, input)),
        project: read::project_summary(store, &root),
        agents: read::agent_caps(store, &root, input),
        channels,
        unjoined: read::unjoined_caps(store, &root, &channel_hs),
        warnings,
        self_pubkey: input.self_pubkey.to_string(),
        self_ref,
        force: input.force,
    };

    ViewInputs {
        meta,
        members: MembersInput {
            roster,
            refs,
            backend,
        },
        presence: PresenceInput { statuses },
        messages: MessagesInput { channels: messages },
    }
}
