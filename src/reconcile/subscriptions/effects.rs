//! Translation from reconciler resource commands to relay transport effects.

use trellis_core::{ResourceCommand, TransactionResult};

use super::keys::id_from_key;
use super::{SubCommand, SubEffect};

pub(super) fn to_effects(result: &TransactionResult<SubCommand>) -> Vec<SubEffect> {
    result
        .resource_plan
        .commands()
        .iter()
        .filter_map(|command| match command {
            ResourceCommand::Open { command, .. } => Some(SubEffect::Open {
                id: command.id.clone(),
                filter: command.filter.clone(),
            }),
            ResourceCommand::Replace { command, .. } | ResourceCommand::Refresh { command, .. } => {
                Some(SubEffect::Replace {
                    id: command.id.clone(),
                    filter: command.filter.clone(),
                })
            }
            ResourceCommand::Close { key, .. } => {
                id_from_key(key).map(|id| SubEffect::Close { id })
            }
        })
        .collect()
}
