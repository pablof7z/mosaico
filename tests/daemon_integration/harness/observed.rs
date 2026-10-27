use super::*;

pub(crate) fn observed_channel_h(parent_h: &str, name: &str) -> Option<String> {
    mosaico::daemon::blocking::call(
        "channel_resolve",
        serde_json::json!({
            "channel": parent_h,
            "name": name,
            "create_if_absent": false,
        }),
    )
    .ok()?
    .get("channel_h")?
    .as_str()
    .map(str::to_string)
}

pub(crate) fn observed_channel_members(path: &str) -> Option<Vec<serde_json::Value>> {
    mosaico::daemon::blocking::call("channel_members", serde_json::json!({ "channel": path }))
        .ok()?
        .get("members")?
        .as_array()
        .cloned()
}

pub(crate) fn observed_channel_has_role(path: &str, pubkey: &str, role: &str) -> bool {
    observed_channel_members(path).is_some_and(|members| {
        members.iter().any(|member| {
            member.get("pubkey").and_then(serde_json::Value::as_str) == Some(pubkey)
                && member.get("role").and_then(serde_json::Value::as_str) == Some(role)
        })
    })
}

pub(crate) fn read_channel_messages(params: serde_json::Value) -> Option<Vec<serde_json::Value>> {
    rt().block_on(async {
        let mut client = mosaico::daemon::client::Client::connect_or_spawn()
            .await
            .ok()?;
        let mut messages = Vec::new();
        client
            .stream("channel_read", params, |message| messages.push(message))
            .await
            .ok()?;
        Some(messages)
    })
}

pub(crate) fn observed_chat(event_id: &str) -> Option<serde_json::Value> {
    read_channel_messages(serde_json::json!({ "id": event_id }))?
        .into_iter()
        .next()
}

pub(crate) fn wait_for_who_session_projection(
    pubkey: &str,
    workspace: &str,
    expected_channels: &std::collections::BTreeSet<String>,
) -> Option<(String, std::collections::BTreeSet<String>)> {
    let mut projection = None;
    wait_until(std::time::Duration::from_secs(25), || {
        projection = rt().block_on(async {
            let mut client = mosaico::daemon::client::Client::connect_or_spawn()
                .await
                .ok()?;
            let value = client
                .call("who", serde_json::json!({ "workspace": workspace }))
                .await
                .ok()?;
            let rows = value["rows"].as_array()?;
            let ours = rows
                .iter()
                .filter(|row| row["pubkey"].as_str() == Some(pubkey))
                .collect::<Vec<_>>();
            let slug = ours.first()?.get("slug")?.as_str()?.to_string();
            let channels = ours
                .iter()
                .map(|row| row["channel"].as_str().map(str::to_string))
                .collect::<Option<std::collections::BTreeSet<_>>>()?;
            Some((slug, channels))
        });
        projection
            .as_ref()
            .is_some_and(|(_, channels)| channels == expected_channels)
    })
    .then_some(projection)
    .flatten()
}

pub(crate) fn session_identity_pubkey(
    store: &mosaico::state::Store,
    pubkey: &str,
) -> Option<String> {
    store.session_identity(pubkey).unwrap().map(|i| i.pubkey)
}

pub(crate) fn pubkey_for_harness_session(
    store: &mosaico::state::Store,
    harness: &str,
    harness_session: &str,
) -> Option<String> {
    store
        .resolve_pubkey_by_locator(harness, "native_resume", harness_session)
        .unwrap()
}

pub(crate) fn session_for_harness_session(
    store: &mosaico::state::Store,
    harness: &str,
    harness_session: &str,
) -> mosaico::state::Session {
    let pubkey = pubkey_for_harness_session(store, harness, harness_session)
        .expect("harness session locator");
    store.get_session(&pubkey).unwrap().expect("session row")
}

pub(crate) fn session_routes(store: &mosaico::state::Store, pubkey: &str) -> Vec<String> {
    store
        .list_session_routes(pubkey)
        .expect("session routes")
        .into_iter()
        .map(|(channel_h, _)| channel_h)
        .collect()
}

pub(crate) fn only_session_route(store: &mosaico::state::Store, pubkey: &str) -> String {
    let routes = session_routes(store, pubkey);
    assert_eq!(routes.len(), 1, "expected exactly one session route");
    routes.into_iter().next().unwrap()
}
