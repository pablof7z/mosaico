use super::model::{MemberRow, PresenceRow};
use super::refs::pubkey_ref;
use super::FabricContextInput;
use crate::state::{Status, Store};
use crate::util::relative_time;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn member_rows(
    store: &Store,
    channel: &str,
    input: &FabricContextInput<'_>,
) -> Vec<MemberRow> {
    let statuses = status_map(store, channel, input.now);
    let mut pubkeys = store
        .list_channel_members(channel)
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.pubkey)
        .collect::<BTreeSet<_>>();
    pubkeys.extend(statuses.keys().cloned());
    if !input.self_pubkey.is_empty() {
        pubkeys.insert(input.self_pubkey.to_string());
    }
    pubkeys
        .into_iter()
        .filter(|pk| !is_backend(store, pk))
        .map(|pk| {
            let status = statuses
                .get(&pk)
                .map(status_text)
                .unwrap_or_else(|| "offline".to_string());
            let seen = statuses
                .get(&pk)
                .map(|s| relative_time(s.last_seen, input.now))
                .unwrap_or_else(|| "unknown".to_string());
            MemberRow {
                reference: if pk == input.self_pubkey {
                    crate::idref::agent_ref_from(
                        input.self_slug,
                        input.local_host,
                        input.local_host,
                    )
                } else {
                    pubkey_ref(store, &pk, input.local_host)
                },
                status,
                seen,
            }
        })
        .collect()
}

pub(super) fn presence_rows(
    store: &Store,
    channel: &str,
    input: &FabricContextInput<'_>,
) -> Vec<PresenceRow> {
    if input.cursor == 0 {
        return Vec::new();
    }
    store
        .live_status_for_channel(channel, input.now)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.updated_at > input.cursor)
        .filter(|s| s.pubkey != input.self_pubkey)
        .map(|s| PresenceRow {
            reference: pubkey_ref(store, &s.pubkey, input.local_host),
            status: status_text(&s),
            seen: relative_time(s.last_seen, input.now),
        })
        .collect()
}

fn status_map(store: &Store, channel: &str, now: u64) -> BTreeMap<String, Status> {
    store
        .live_status_for_channel(channel, now)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.pubkey.clone(), s))
        .collect()
}

fn status_text(status: &Status) -> String {
    if status.busy {
        return non_empty(&status.activity)
            .or_else(|| non_empty(&status.title))
            .unwrap_or_else(|| "working".to_string());
    }
    non_empty(&status.title).unwrap_or_else(|| "idle".to_string())
}

fn is_backend(store: &Store, pubkey: &str) -> bool {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.is_backend)
        .unwrap_or(false)
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}
