use super::*;
use std::collections::BTreeSet;

pub(super) struct DirectMention<'a> {
    pub(super) event_id: &'a str,
    pub(super) from_pubkey: &'a str,
    pub(super) channel_h: &'a str,
    pub(super) body: &'a str,
    pub(super) created_at: u64,
    pub(super) target_pubkeys: &'a [String],
    pub(super) attachments: &'a [crate::domain::ChatAttachment],
}

pub(super) struct RouteReport {
    pub(super) owned_targets: Vec<String>,
}

/// Persist and schedule one direct mention using pubkey ownership alone.
/// Relay admission has already decided whether an inbound sender may write in
/// the channel; routes and runtime state only affect later execution.
pub(super) fn route(state: &Arc<DaemonState>, mention: DirectMention<'_>) -> Result<RouteReport> {
    let owned = owned_pubkeys(state)?;
    let targets = mention
        .target_pubkeys
        .iter()
        .filter(|target| target.as_str() != mention.from_pubkey)
        .filter(|target| owned.contains(target.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let mut newly_parked = false;
    for target in &targets {
        newly_parked |= state.with_store(|store| {
            store.park_direct_mention(
                mention.event_id,
                target,
                mention.from_pubkey,
                mention.channel_h,
                mention.body,
                mention.created_at,
            )
        })?;
    }

    if newly_parked {
        crate::session_host::ring_doorbells(state.clone());
    }
    if !targets.is_empty() {
        let chat = ChatMessage {
            from: crate::domain::AgentRef::new(mention.from_pubkey, String::new()),
            channel: mention.channel_h.to_string(),
            body: mention.body.to_string(),
            mentioned_pubkeys: targets.clone(),
            attachments: mention.attachments.to_vec(),
        };
        super::demux::dispatch_offline_mentions(state, mention.event_id, &chat, &targets);
    }

    Ok(RouteReport {
        owned_targets: targets,
    })
}

fn owned_pubkeys(state: &DaemonState) -> Result<BTreeSet<String>> {
    let mut owned = state.hosted_pubkeys().into_iter().collect::<BTreeSet<_>>();
    owned.extend(crate::identity::list_local_pubkeys(
        &crate::config::mosaico_home(),
    ));
    owned.extend(state.with_store(|store| store.list_owned_session_pubkeys())?);
    Ok(owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::LocalAgentUpdate;
    use crate::state::{RecordMessage, RegisterSession};
    use crate::test_env::EnvGuard;
    use nostr::Keys;

    #[tokio::test(flavor = "current_thread")]
    async fn ownership_router_parks_stable_and_revoked_targets_but_ignores_remote() {
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
        let (stable, _) = crate::identity::save_local_agent(
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
        let stable_pk = stable.pubkey_hex().unwrap();
        let revoked_pk = Keys::generate().public_key().to_hex();
        let remote_pk = Keys::generate().public_key().to_hex();
        let sender_pk = Keys::generate().public_key().to_hex();
        let state = DaemonState::new_for_test().await;

        state.with_store(|store| {
            store
                .reserve_hook_session_for_test(&RegisterSession {
                    pubkey: revoked_pk.clone(),
                    observed_harness: "codex".into(),
                    agent_slug: "worker".into(),
                    launch_channel_h: "room".into(),
                    work_root: "room".into(),
                    child_pid: None,
                    now: 2,
                })
                .unwrap();
            let generation = store
                .get_session(&revoked_pk)
                .unwrap()
                .unwrap()
                .runtime_generation;
            store
                .revoke_route_and_mark_absent(&revoked_pk, "room", 3)
                .unwrap();
            store
                .revoke_session_recovery_if_generation(&revoked_pk, generation)
                .unwrap();
            store
                .finalize_session_recovery_revocation(&revoked_pk, generation, 4)
                .unwrap();
        });

        let targets = vec![stable_pk.clone(), revoked_pk.clone(), remote_pk.clone()];
        state.with_store(|store| seed_message(store, "event", &sender_pk));
        let report = route(
            &state,
            DirectMention {
                event_id: "event",
                from_pubkey: &sender_pk,
                channel_h: "room",
                body: "please inspect",
                created_at: 5,
                target_pubkeys: &targets,
                attachments: &[],
            },
        )
        .unwrap();

        assert_eq!(
            report.owned_targets.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([revoked_pk.clone(), stable_pk.clone()])
        );
        state.with_store(|store| {
            assert_eq!(store.peek_pending_for_pubkey(&stable_pk).unwrap().len(), 1);
            assert_eq!(store.peek_pending_for_pubkey(&revoked_pk).unwrap().len(), 1);
            assert!(store
                .peek_pending_for_pubkey(&remote_pk)
                .unwrap()
                .is_empty());
            let recipients = store.message_recipients("event").unwrap();
            assert_eq!(recipients.len(), 2);
            assert!(recipients.iter().all(|edge| {
                edge.recipient_pubkey == stable_pk || edge.recipient_pubkey == revoked_pk
            }));
        });

        let mut retryable = Vec::new();
        for _ in 0..20 {
            tokio::task::yield_now().await;
            retryable = state.with_store(|store| {
                store
                    .list_retryable_offline_mentions(now_secs() + 60, 10)
                    .unwrap()
            });
            if retryable.len() == 2 {
                break;
            }
        }
        assert_eq!(retryable.len(), 2);
        assert!(retryable.iter().all(|claim| {
            claim.event_id == "event"
                && claim.channel_h == "room"
                && claim.body == "please inspect"
                && claim.from_pubkey == sender_pk
        }));
    }

    fn seed_message(store: &Store, event_id: &str, sender_pk: &str) {
        store
            .record_message(&RecordMessage {
                message_id: event_id.into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: sender_pk.into(),
                body: "please inspect".into(),
                created_at: 5,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some(event_id.into()),
                error: None,
            })
            .unwrap();
    }
}
