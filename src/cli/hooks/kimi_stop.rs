use super::{hook_forensics::HookCallLog, HostDef};
use crate::cli::turn::{turn_check, turn_end, EmitFormat};
use anyhow::Result;

pub(super) async fn run(
    host: &HostDef,
    session_id: &str,
    emit: EmitFormat,
    call_log: &HookCallLog,
) -> Result<()> {
    if session_id.is_empty() {
        return Ok(());
    }
    if host.name != "kimi" {
        return turn_end(session_id.to_string()).await;
    }

    // Kimi ignores PostToolUse stdout and permits only one Stop-hook
    // continuation per turn. Deliver any late fabric delta, then close the
    // original turn even when Kimi performs that single continuation.
    let result = turn_check(Some(session_id.to_string()), emit).await?;
    call_log.context_audit(
        host.name,
        "stop",
        Some(session_id),
        result.audit,
        result.context.as_deref(),
    );
    turn_end(session_id.to_string()).await
}
