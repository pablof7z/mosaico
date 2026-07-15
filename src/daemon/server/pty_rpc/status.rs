use super::*;

#[derive(Clone)]
struct PtySessionBinding {
    pubkey: String,
    npub: Option<String>,
    handle: Option<String>,
}

pub(super) async fn rpc_pty_status(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let session_by_pty = pty_session_bindings(state);
    let arr: Vec<serde_json::Value> = crate::pty::read_all_metadata()
        .into_iter()
        .map(|meta| {
            let live = crate::pty::is_live(&meta.id);
            let binding = session_by_pty.get(&meta.id);
            let pubkey = binding.map(|b| b.pubkey.clone());
            let npub = binding.and_then(|b| b.npub.clone());
            let handle = binding.and_then(|b| b.handle.clone());
            serde_json::json!({
                "pty_id": meta.id,
                "pubkey": pubkey,
                "npub": npub,
                "handle": handle,
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
                for locator in s.list_locators_of_kind(crate::state::LOCATOR_PTY)? {
                    if out.contains_key(&locator.locator_value) {
                        continue;
                    }
                    if let Some(rec) = s.get_session(&locator.pubkey)? {
                        let pubkey = rec.pubkey;
                        out.insert(
                            locator.locator_value,
                            PtySessionBinding {
                                npub: crate::idref::npub(&pubkey),
                                handle: s.handle_for_pubkey(&pubkey)?,
                                pubkey,
                            },
                        );
                    }
                }
                Ok(out)
            },
        )
        .unwrap_or_default()
}
