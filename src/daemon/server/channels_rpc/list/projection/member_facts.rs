use anyhow::Result;

use crate::agent_count::{MemberFactIndex, MemberFacts};
use crate::state::Store;

pub(super) fn capture(
    store: &Store,
    channel: &str,
    index: &MemberFactIndex,
) -> Result<(bool, Vec<MemberFacts>)> {
    let hydrated = store.group_state_available(channel)?;
    let members = store
        .list_channel_members(channel)?
        .iter()
        .map(|member| index.normalize(store, member))
        .collect::<Result<Vec<_>>>()?;
    Ok((hydrated, members))
}
