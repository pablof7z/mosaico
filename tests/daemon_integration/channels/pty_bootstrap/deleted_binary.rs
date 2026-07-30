use super::*;

#[cfg(target_os = "linux")]
struct MosaicoBinGuard(Option<std::ffi::OsString>);

#[cfg(target_os = "linux")]
impl MosaicoBinGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("MOSAICO_BIN");
        unsafe { std::env::set_var("MOSAICO_BIN", path) };
        Self(previous)
    }
}

#[cfg(target_os = "linux")]
impl Drop for MosaicoBinGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(path) => unsafe { std::env::set_var("MOSAICO_BIN", path) },
            None => unsafe { std::env::remove_var("MOSAICO_BIN") },
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn replaced_daemon_binary_still_spawns_its_pty_supervisor() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, false);

    let channel = unique_session("deleted-daemon-binary");
    let work_dir = home.dir.path().join(&channel);
    add_workspace_mapping(&home, &channel, &work_dir);
    let agent = "deleted-binary-agent";
    configure_pty_agent(&home, agent, "forever");

    let daemon_bin = home.dir.path().join("mosaico-daemon");
    std::fs::copy(bin(), &daemon_bin).unwrap();
    let _bin_guard = MosaicoBinGuard::set(&daemon_bin);
    let pty_id = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        std::fs::remove_file(&daemon_bin).unwrap();
        let response = client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": agent,
                    "root": &channel,
                    "channel": &channel,
                    "cwd": &work_dir,
                }),
            )
            .await
            .expect("daemon with a replaced binary must spawn its supervisor");
        response["pty_id"].as_str().expect("pty_id").to_string()
    });

    let metadata = pty_meta(&pty_id);
    assert_eq!(metadata.command, ["opencode", "forever"]);
    mosaico::pty::kill(&pty_id).unwrap();
    stop_daemon(&home);
}
