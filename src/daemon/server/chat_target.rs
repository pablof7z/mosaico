use super::resolution::work_root_for;
use super::*;

pub(in crate::daemon::server) struct ChatTarget {
    pub channel_h: String,
    pub explicit: bool,
}

pub(in crate::daemon::server) fn resolve_chat_target(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    explicit: Option<&str>,
    command: &str,
) -> Result<ChatTarget> {
    if let Some(reference) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let root = state.with_store(|s| super::root_channel(s, &rec.channel_h));
        let channel_h = state.with_store(|s| resolve_chat_channel_ref(s, &root, reference))?;
        return Ok(ChatTarget {
            channel_h,
            explicit: true,
        });
    }

    let joined = state.with_store(|s| s.list_session_joined_channels(&rec.session_id))?;
    match joined.as_slice() {
        [] => Ok(ChatTarget {
            channel_h: rec.channel_h.clone(),
            explicit: false,
        }),
        [(channel_h, _)] => Ok(ChatTarget {
            channel_h: channel_h.clone(),
            explicit: false,
        }),
        _ => {
            let refs = state.with_store(|s| {
                joined
                    .iter()
                    .map(|(h, _)| chat_channel_reference(s, h))
                    .collect::<Vec<_>>()
            });
            anyhow::bail!(
                "{} is ambiguous because this session is joined to {} channels. \
Pass one explicitly:\n{}",
                command,
                joined.len(),
                refs.iter()
                    .map(|r| format!("  tenex-edge {command} --channel {r}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}

/// Like [`resolve_chat_target`] but with `mkdir -p` semantics for an explicit
/// `--channel` reference: when the channel-relative path does not exist yet,
/// create the whole missing ancestor chain (not just the leaf) and target the
/// leaf. The non-explicit (joined-channel inference) path is unchanged.
pub(in crate::daemon::server) async fn resolve_chat_target_provisioning(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    explicit: Option<&str>,
    command: &str,
) -> Result<ChatTarget> {
    if let Some(reference) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let root = state.with_store(|s| super::root_channel(s, &rec.channel_h));
        match state.with_store(|s| super::resolve_channel_ref(s, &root, reference)) {
            super::ChannelResolution::Unique(channel_h) => {
                return Ok(ChatTarget {
                    channel_h,
                    explicit: true,
                })
            }
            super::ChannelResolution::Ambiguous(refs) => anyhow::bail!(
                "channel reference {reference:?} is ambiguous; re-run with one of: {}",
                refs.into_iter()
                    .map(|r| format!("--channel {r}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            super::ChannelResolution::NotFound => {
                // mkdir -p: provision the missing chain and target the leaf.
                let channel_h = super::resolve_channel_path(state, &root, reference, true).await?;
                return Ok(ChatTarget {
                    channel_h,
                    explicit: true,
                });
            }
        }
    }
    resolve_chat_target(state, rec, None, command)
}

fn resolve_chat_channel_ref(
    store: &crate::state::Store,
    root: &str,
    reference: &str,
) -> Result<String> {
    match super::resolve_channel_ref(store, root, reference) {
        super::ChannelResolution::Unique(h) => Ok(h),
        super::ChannelResolution::Ambiguous(refs) => anyhow::bail!(
            "channel reference {reference:?} is ambiguous; re-run with one of: {}",
            refs.into_iter()
                .map(|r| format!("--channel {r}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        super::ChannelResolution::NotFound => {
            anyhow::bail!("no channel matching {reference:?} in this channel")
        }
    }
}

fn chat_channel_reference(store: &crate::state::Store, channel_h: &str) -> String {
    let root = work_root_for(store, channel_h);
    if root == channel_h {
        return channel_h.to_string();
    }
    format!("@{}", &channel_h[..channel_h.len().min(8)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Session, Store};

    fn session(channel_h: &str) -> Session {
        Session {
            session_id: "sess".to_string(),
            agent_pubkey: "pk".to_string(),
            agent_slug: "codex".to_string(),
            channel_h: channel_h.to_string(),
            harness: "codex".to_string(),
            child_pid: None,
            transcript_path: None,
            alive: true,
            created_at: 1,
            last_seen: 1,
            working: false,
            turn_started_at: 0,
            last_distill_at: 0,
            seen_cursor: 0,
            title: String::new(),
            activity: String::new(),
            resume_id: String::new(),
        }
    }

    #[test]
    fn explicit_chat_target_resolves_channel_relative_path() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        store
            .upsert_channel("a1111111", "epic", "", "root", 1)
            .unwrap();
        store
            .upsert_channel("b2222222", "planning", "", "a1111111", 1)
            .unwrap();

        let resolved = resolve_chat_channel_ref(&store, "root", "epic/planning").unwrap();
        assert_eq!(resolved, "b2222222");
    }

    #[test]
    fn explicit_chat_target_resolves_name_and_literal_id() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        store
            .upsert_channel("abcd1234", "planning", "", "root", 1)
            .unwrap();

        assert_eq!(
            resolve_chat_channel_ref(&store, "root", "planning").unwrap(),
            "abcd1234"
        );
        assert_eq!(
            resolve_chat_channel_ref(&store, "root", "abcd1234").unwrap(),
            "abcd1234"
        );
    }

    #[test]
    fn multi_join_without_explicit_channel_errors_with_reruns() {
        let store = Store::open_memory().unwrap();
        let rec = session("root");
        store
            .upsert_session_row(
                "sess",
                &crate::state::RegisterSession {
                    harness: "codex".to_string(),
                    external_id_kind: "harness_session".to_string(),
                    external_id: "sess".to_string(),
                    agent_pubkey: "pk".to_string(),
                    agent_slug: "codex".to_string(),
                    channel_h: "root".to_string(),
                    child_pid: None,
                    transcript_path: None,
                    resume_id: String::new(),
                    now: 1,
                },
            )
            .unwrap();
        store.join_session_channel("sess", "root", 1).unwrap();
        store.join_session_channel("sess", "other", 2).unwrap();

        let joined = store.list_session_joined_channels("sess").unwrap();
        assert_eq!(joined.len(), 2);
        let refs = joined
            .iter()
            .map(|(h, _)| chat_channel_reference(&store, h))
            .collect::<Vec<_>>();
        assert!(refs.contains(&"root".to_string()));
        assert!(refs.contains(&"other".to_string()));
        assert_eq!(rec.channel_h, "root");
    }
}
