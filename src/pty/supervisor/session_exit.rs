pub(super) struct SessionExitGuard {
    pty_id: String,
}

impl SessionExitGuard {
    pub(super) fn new(pty_id: String) -> Self {
        Self { pty_id }
    }
}

impl Drop for SessionExitGuard {
    fn drop(&mut self) {
        notify_daemon(self.pty_id.clone());
    }
}

fn notify_daemon(pty_id: String) {
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            let Ok(mut client) = crate::daemon::client::Client::connect_running().await else {
                return;
            };
            client
                .call(
                    "pty_supervisor_exit",
                    serde_json::json!({
                        "pty_id": pty_id,
                        "pubkey": std::env::var("MOSAICO_PUBKEY").ok(),
                    }),
                )
                .await
                .ok();
        });
    })
    .join()
    .ok();
}
