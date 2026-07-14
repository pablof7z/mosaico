//! Pure mapping from a covered entity to its resource key, semantic id, narrow
//! filter, and the shared planner. No graph state lives here.

use nostr_sdk::prelude::{Filter, SubscriptionId};
use trellis_core::{PlanContext, PlanError, ResourceKey, ResourcePlan, SetDiff};

use crate::fabric::subscriptions::{
    global_kind_filter, id_global_kind, id_gstate_narrow, id_h_narrow, id_p_narrow,
    narrow_gstate_filter, narrow_h_filter, narrow_p_filter,
};

/// The tag-space a covered entity belongs to. Each maps to one narrow filter
/// shape and one id-namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Space {
    /// Daemon-lifetime discovery stream with no tag scope.
    GlobalKind,
    /// Channel chat/status/long-form scoped by `#h`.
    ChannelH,
    /// Relay-authored group state (39000/39001/39002) scoped by `#d`.
    GroupStateD,
    /// Chat/long-form addressed by `#p`.
    PubkeyP,
}

impl Space {
    fn as_seg(self) -> &'static str {
        match self {
            Space::GlobalKind => "k",
            Space::ChannelH => "h",
            Space::GroupStateD => "d",
            Space::PubkeyP => "p",
        }
    }
    fn from_seg(seg: &str) -> Option<Self> {
        match seg {
            "k" => Some(Space::GlobalKind),
            "h" => Some(Space::ChannelH),
            "d" => Some(Space::GroupStateD),
            "p" => Some(Space::PubkeyP),
            _ => None,
        }
    }
}

/// One covered entity: its tag-space plus its identifier (a channel/group id or
/// a pubkey). The set-collection key the planner diffs.
pub(super) type SubKey = (Space, String);

/// In-graph command payload: the narrow filter + semantic id for an Open/Replace.
#[derive(Clone, Debug, PartialEq)]
pub struct SubCommand {
    /// Semantic subscription id for this entity's REQ.
    pub id: SubscriptionId,
    /// The narrow relay filter for this entity.
    pub filter: Filter,
}

/// Resource identity for one covered entity: `sub/<space>/<entity>`.
pub fn sub_key(space: Space, entity: &str) -> ResourceKey {
    ResourceKey::from_segments(["sub", space.as_seg(), entity])
}

/// Semantic subscription id for one covered entity.
fn sub_id(space: Space, entity: &str) -> SubscriptionId {
    match space {
        Space::GlobalKind => id_global_kind(entity.parse().expect("global kind is numeric")),
        Space::ChannelH => id_h_narrow(entity),
        Space::GroupStateD => id_gstate_narrow(entity),
        Space::PubkeyP => id_p_narrow(entity),
    }
}

/// The narrow relay filter for one covered entity.
fn sub_filter(space: Space, entity: &str) -> Filter {
    match space {
        Space::GlobalKind => global_kind_filter(entity.parse().expect("global kind is numeric")),
        Space::ChannelH => narrow_h_filter(entity),
        Space::GroupStateD => narrow_gstate_filter(entity),
        Space::PubkeyP => narrow_p_filter(entity),
    }
}

/// The in-graph Open/Replace command payload for one covered entity.
pub(super) fn sub_command(space: Space, entity: &str) -> SubCommand {
    SubCommand {
        id: sub_id(space, entity),
        filter: sub_filter(space, entity),
    }
}

/// Reconstruct the semantic subscription id from a close command's resource key.
/// Closes carry no payload, so the id is recovered from the `sub/<space>/<entity>`
/// segments.
pub(super) fn id_from_key(key: &ResourceKey) -> Option<SubscriptionId> {
    let space = Space::from_seg(key.segment(1)?)?;
    Some(sub_id(space, key.segment(2)?))
}

/// Shared planner for both the daemon scope and every session scope: open added
/// entities, close removed ones. Refcounting across scopes is the resource
/// reconciler's job — this only reports what THIS scope wants.
pub(super) fn plan_subs(
    ctx: &PlanContext<SetDiff<SubKey>>,
) -> Result<ResourcePlan<SubCommand>, PlanError> {
    let mut plan = ResourcePlan::new();
    for added in &ctx.diff().added {
        let (space, entity) = &added.value;
        plan.open(
            sub_key(*space, entity),
            ctx.scope(),
            sub_command(*space, entity),
        );
    }
    for removed in &ctx.diff().removed {
        let (space, entity) = &removed.value;
        plan.close(sub_key(*space, entity), ctx.scope());
    }
    Ok(plan)
}
