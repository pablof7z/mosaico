use super::*;
use nostr::Keys;
use std::collections::BTreeSet;
use std::time::Duration;

#[test]
fn child_copies_every_parent_admin_in_one_group_event() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let relay = shared_nip29_relay_url();
    let creator = start_creator(&home, &unique_session("admin-copy-creator"));
    let additional_admins = [
        Keys::generate().public_key().to_hex(),
        Keys::generate().public_key().to_hex(),
    ];
    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        for pubkey in &additional_admins {
            client
                .call(
                    "channel_add_member",
                    serde_json::json!({
                        "channel": "#tmp",
                        "pubkey": pubkey,
                        "admin": true,
                        "session": &creator,
                    }),
                )
                .await
                .expect("add parent administrator");
        }
    });
    assert!(wait_until(Duration::from_secs(15), || {
        refresh_channel_members("#tmp");
        let store = Store::open(&home.store_path()).unwrap();
        additional_admins
            .iter()
            .all(|pubkey| store.is_channel_admin("tmp", pubkey).unwrap_or(false))
    }));
    let parent_events = management_events(&relay, "tmp");
    for pubkey in &additional_admins {
        assert_eq!(
            events_naming(&parent_events, pubkey),
            1,
            "one channel-add command must publish that administrator exactly once"
        );
    }
    let child_name = unique_session("admin-copy-child");

    let created = rt().block_on(async {
        Client::connect_or_spawn()
            .await
            .expect("connect")
            .call(
                "channel_create",
                serde_json::json!({
                    "channel": format!("#tmp/{child_name}"),
                    "about": "inherits every parent administrator",
                    "agents": [],
                    "session": &creator,
                }),
            )
            .await
            .expect("create child")
    });
    assert_eq!(created["channel"], format!("#tmp/{child_name}"));

    let child_h = named_child_h(&home, "tmp", &child_name);
    let backend = pubkey_of(EXAMPLE_BACKEND_SEC_HEX);
    let expected = BTreeSet::from([
        pubkey_of(EXAMPLE_USER_NSEC),
        additional_admins[0].clone(),
        additional_admins[1].clone(),
    ]);
    let events = management_events(&relay, &child_h);
    let admin_batches = events
        .iter()
        .filter(|event| event["pubkey"].as_str() == Some(&backend))
        .filter_map(admin_pubkeys)
        .collect::<Vec<_>>();

    assert_eq!(
        admin_batches,
        vec![expected],
        "the child must inherit every parent admin through one kind:9000 event; events={events:#?}"
    );
    stop_daemon(&home);
}

fn management_events(relay: &str, group: &str) -> Vec<serde_json::Value> {
    let output = std::process::Command::new(crate::common::nak_bin())
        .args(["req", "-k", "9000", "-h", group, relay])
        .output()
        .expect("query kind:9000 events");
    assert!(
        output.status.success(),
        "nak kind:9000 query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|event: &serde_json::Value| event["kind"].as_u64() == Some(9000))
        .collect()
}

fn admin_pubkeys(event: &serde_json::Value) -> Option<BTreeSet<String>> {
    let pubkeys = event["tags"]
        .as_array()?
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_array()?;
            (parts.first()?.as_str()? == "p" && parts.get(2)?.as_str()? == "admin")
                .then(|| parts.get(1)?.as_str().map(str::to_string))
                .flatten()
        })
        .collect::<BTreeSet<_>>();
    (!pubkeys.is_empty()).then_some(pubkeys)
}

fn events_naming(events: &[serde_json::Value], pubkey: &str) -> usize {
    events
        .iter()
        .filter(|event| {
            event["tags"].as_array().is_some_and(|tags| {
                tags.iter().any(|tag| {
                    tag.as_array().is_some_and(|parts| {
                        parts.first().and_then(serde_json::Value::as_str) == Some("p")
                            && parts.get(1).and_then(serde_json::Value::as_str) == Some(pubkey)
                    })
                })
            })
        })
        .count()
}
