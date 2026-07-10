use super::*;

#[derive(Clone)]
struct PtySessionBinding {
    session_id: String,
    display_id: Option<String>,
}

pub(super) async fn rpc_pty_status(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let session_by_pty = pty_session_bindings(state);
    let arr: Vec<serde_json::Value> = crate::pty::read_all_metadata()
        .into_iter()
        .map(|meta| {
            let live = crate::pty::is_live(&meta.id);
            let binding = session_by_pty.get(&meta.id);
            let session_id = binding.map(|b| b.session_id.clone());
            let display_id = binding.and_then(|b| b.display_id.clone());
            serde_json::json!({
                "pty_id": meta.id,
                "display_id": display_id,
                "session_id": session_id,
                "socket": meta.socket,
                "agent": meta.agent,
                "root": meta.root,
                "cwd": meta.cwd,
                "command": meta.command,
                "live": live,
            })
        })
        .collect();
    Ok(serde_json::json!({ "endpoints": arr }))
}

fn pty_session_bindings(
    state: &Arc<DaemonState>,
) -> std::collections::HashMap<String, PtySessionBinding> {
    state
        .with_store(
            |s| -> Result<std::collections::HashMap<String, PtySessionBinding>> {
                let mut out = std::collections::HashMap::new();
                for alias in s.list_aliases_of_kind("pty_session")? {
                    if out.contains_key(&alias.external_id) {
                        continue;
                    }
                    let display_id = s.get_session(&alias.session_id)?.map(|rec| {
                        s.session_identity_for_session(&rec.session_id)
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| {
                                crate::identity::SessionIdentity::fallback(
                                    &rec.session_id,
                                    rec.agent_slug,
                                    rec.agent_pubkey,
                                )
                            })
                            .display_slug()
                    });
                    out.insert(
                        alias.external_id,
                        PtySessionBinding {
                            session_id: alias.session_id,
                            display_id,
                        },
                    );
                }
                Ok(out)
            },
        )
        .unwrap_or_default()
}
