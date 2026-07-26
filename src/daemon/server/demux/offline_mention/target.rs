use crate::daemon::server::DaemonState;
use std::sync::Arc;

pub(super) struct MentionTarget {
    pub(super) agent_slug: String,
    pub(super) session: Option<crate::state::Session>,
}

pub(super) enum Resolution {
    Ready(Box<MentionTarget>),
    Retry,
    Reject,
}

pub(super) fn resolve_and_persist(
    state: &Arc<DaemonState>,
    event_id: &str,
    mentioned_pubkey: &str,
    channel: &str,
    body: &str,
    requester_pubkey: Option<&str>,
) -> Resolution {
    let session = match state.with_store(|store| store.get_session(mentioned_pubkey)) {
        Ok(session) => session,
        Err(error) => {
            tracing::error!(pubkey = %mentioned_pubkey, channel, %error, "exact mention target lookup failed");
            return Resolution::Retry;
        }
    };
    let configured_slug = match configured_agent_slug(state, mentioned_pubkey) {
        Ok(slug) => slug,
        Err(error) => {
            tracing::error!(
                pubkey = %mentioned_pubkey,
                channel,
                error = %format!("{error:#}"),
                "exact mention agent inventory lookup failed"
            );
            None
        }
    };
    let Some(agent_slug) = session
        .as_ref()
        .map(|session| session.agent_slug.clone())
        .or(configured_slug)
    else {
        tracing::warn!(
            event_id,
            pubkey = %mentioned_pubkey,
            channel,
            "exact mention target has no locally owned session or configured identity"
        );
        return Resolution::Reject;
    };

    let admitted = state.with_store(|store| {
        store
            .session_membership_admits_event(mentioned_pubkey, channel, event_id)
            .unwrap_or(false)
    });
    if !admitted {
        tracing::warn!(
            event_id,
            pubkey = %mentioned_pubkey,
            channel,
            "exact mention predates the target's durable channel membership; refusing recovery"
        );
        return Resolution::Reject;
    }

    let created_at = state.with_store(|store| {
        store
            .get_event(event_id)
            .ok()
            .flatten()
            .map(|event| event.created_at)
            .unwrap_or_default()
    });
    let persisted = state.with_store(|store| {
        store.enqueue_inbox(
            event_id,
            mentioned_pubkey,
            requester_pubkey.unwrap_or_default(),
            channel,
            body,
            created_at,
        )?;
        store.add_message_recipient(event_id, mentioned_pubkey, None)?;
        Ok::<_, anyhow::Error>(())
    });
    if let Err(error) = persisted {
        tracing::error!(
            event_id,
            pubkey = %mentioned_pubkey,
            channel,
            %error,
            "exact mention could not be persisted before recovery; refusing launch"
        );
        return Resolution::Retry;
    }
    Resolution::Ready(Box::new(MentionTarget {
        agent_slug,
        session,
    }))
}

fn configured_agent_slug(state: &DaemonState, pubkey: &str) -> anyhow::Result<Option<String>> {
    Ok(state
        .agent_inventory(None)?
        .durable_agent_for_pubkey(pubkey)
        .map(|agent| agent.agent_slug.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::LocalAgentUpdate;
    use crate::test_env::EnvGuard;

    #[tokio::test]
    async fn rejected_inventory_record_cannot_resolve_through_the_keystore() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join(".mosaico");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("harnesses.json"),
            r#"{"codex-pty":{"harness":"codex","transport":"pty"}}"#,
        )
        .unwrap();
        let mut env = EnvGuard::set("HOME", root.path());
        env.set_var("MOSAICO_HOME", &home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
        let (configured, _) = crate::identity::save_local_agent(
            &home,
            "writer",
            LocalAgentUpdate {
                harness: "codex-pty".into(),
                profile: None,
                per_session_key: Some(false),
                byline: None,
            },
            1,
        )
        .unwrap();
        let pubkey = configured.pubkey_hex().unwrap();
        let state = DaemonState::new_for_test().await;
        assert_eq!(
            configured_agent_slug(&state, &pubkey).unwrap().as_deref(),
            Some("writer")
        );

        crate::identity::save_local_agent(
            &home,
            "writer",
            LocalAgentUpdate {
                harness: "missing-bundle".into(),
                profile: None,
                per_session_key: None,
                byline: None,
            },
            2,
        )
        .unwrap();

        assert_eq!(configured_agent_slug(&state, &pubkey).unwrap(), None);
    }
}
