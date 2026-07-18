use super::data::AgentRow;
use crate::cli::interactive::agent_picker::DeleteScope;
use anyhow::Result;

/// Confirmation already happened inline in the picker's status bar (see
/// `agent_picker::picker::PendingDelete`); this just performs the resolved
/// deletion.
pub(super) async fn delete(row: &AgentRow, scope: DeleteScope) -> Result<()> {
    let profile = row.native_profile.as_ref();
    if matches!(scope, DeleteScope::Agent | DeleteScope::Both)
        && super::remove_agent_config(&row.slug).await?
    {
        println!("Deleted agent configuration {}", row.slug);
    }
    if matches!(scope, DeleteScope::Profile | DeleteScope::Both) {
        if let Some(profile) = profile {
            if crate::agent_catalog::remove_native_profile(profile)? {
                println!("Deleted native profile {}", profile.path.display());
            }
        }
    }
    super::schedule_roster_refresh(Some(&row.slug)).await;
    Ok(())
}
