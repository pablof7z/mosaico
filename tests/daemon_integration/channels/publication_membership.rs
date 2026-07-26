use super::*;
use std::time::Duration;

#[test]
fn status_publication_never_rejoins_after_explicit_last_route_leave() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new().with_backend_key();
    let channel = unique_session("publish-does-not-rejoin");
    let cwd = home.dir.path().join(&channel);
    std::fs::create_dir_all(&cwd).unwrap();
    initialize_workspace_root(&channel, cwd.to_str().unwrap());
    let sid = unique_session("sole-route");
    let watch_pid = std::process::id() as i32;

    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "coder",
                        "harness_session": &sid,
                        "cwd": &cwd,
                        "channel": &channel,
                        "watch_pid": watch_pid,
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("session_start");
    });

    let pubkey = {
        let store = Store::open(&home.store_path()).unwrap();
        pubkey_for_harness_session(&store, "claude-code", &sid).unwrap()
    };
    if !wait_until(Duration::from_secs(25), || {
        refresh_channel_members(&format!("/{channel}"));
        let store = Store::open(&home.store_path()).unwrap();
        store.has_session_route(&pubkey, &channel).unwrap_or(false)
            && store
                .list_channel_members(&channel)
                .unwrap_or_default()
                .iter()
                .any(|member| member.pubkey == pubkey)
            && !store
                .latest_receipts_for_surface("status", 1)
                .unwrap_or_default()
                .is_empty()
    }) {
        let store = Store::open(&home.store_path()).unwrap();
        panic!(
            "session did not establish its sole route, membership, and presence; \
             routes={:?}; members={:?}; receipts={:?}; standing={:?}; daemon_log={}",
            store.list_session_routes(&pubkey).unwrap_or_default(),
            store.list_channel_members(&channel).unwrap_or_default(),
            store
                .latest_receipts_for_surface("status", 5)
                .unwrap_or_default(),
            store.get_session_standing(&pubkey, &channel).ok().flatten(),
            std::fs::read_to_string(home.dir.path().join("daemon.log"))
                .unwrap_or_else(|error| format!("<{error}>")),
        );
    }
    let before_receipt = Store::open(&home.store_path())
        .unwrap()
        .latest_receipts_for_surface("status", 1)
        .unwrap()[0]
        .id;

    rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let left = client
            .call(
                "channel_leave",
                serde_json::json!({
                    "channel": format!("/{channel}"),
                    "harness": "claude-code",
                    "watch_pid": watch_pid,
                    "agent": "coder",
                    "cwd": &cwd,
                }),
            )
            .await
            .expect("leave sole route");
        assert_eq!(left["left"].as_bool(), Some(true));
    });

    assert!(
        wait_until(Duration::from_secs(15), || {
            Store::open(&home.store_path())
                .unwrap()
                .latest_receipts_for_surface("status", 1)
                .unwrap_or_default()
                .first()
                .is_some_and(|receipt| {
                    receipt.id > before_receipt && receipt.commands.contains("expire")
                })
        }),
        "presence publisher did not process the post-leave expiry"
    );
    refresh_channel_members(&format!("/{channel}"));
    let store = Store::open(&home.store_path()).unwrap();
    assert!(store.list_session_routes(&pubkey).unwrap().is_empty());
    assert_eq!(
        store
            .get_session_standing(&pubkey, &channel)
            .unwrap()
            .unwrap()
            .state,
        mosaico::state::StandingState::Absent
    );
    assert!(
        store
            .list_channel_members(&channel)
            .unwrap()
            .iter()
            .all(|member| member.pubkey != pubkey),
        "relay-confirmed member mirror must remain absent after expiry publication"
    );
    stop_daemon(&home);
}
