use anyhow::{bail, Result};
use std::time::{Duration, Instant};

/// How long `stop` waits for a shut-down daemon to actually exit (release its
/// startup flock) before giving up and reporting it as still-running.
const DAEMON_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn stop() -> Result<()> {
    request_shutdown();
    crate::daemon::set_inhibit();
    eprintln!(
        "[mosaico] hooks will not restart the daemon; \
         run `mosaico daemon restart` to resume"
    );
    Ok(())
}

pub(super) async fn restart() -> Result<()> {
    if !request_shutdown() {
        bail!("daemon shutdown did not complete; refusing to start a second daemon")
    }

    crate::daemon::clear_inhibit();
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call("ping", serde_json::json!({})).await?;
    eprintln!("[mosaico] daemon restarted");
    Ok(())
}

/// Ask a running daemon to exit without spawning one. Returns whether it is
/// safe for a caller to start a replacement daemon.
fn request_shutdown() -> bool {
    match crate::daemon::blocking::call_no_spawn("shutdown", serde_json::json!({})) {
        Ok(_) => wait_for_exit(),
        Err(_) => {
            eprintln!("[mosaico] daemon was not running");
            true
        }
    }
}

/// The RPC layer acks `shutdown` before the daemon has torn down its relay
/// connection, socket, and startup flock. Poll that flock so `stop` confirms
/// the process really exited.
fn wait_for_exit() -> bool {
    let deadline = Instant::now() + DAEMON_SHUTDOWN_TIMEOUT;
    loop {
        match crate::daemon::client::StartupLock::try_acquire() {
            Ok(Some(_lock)) => {
                eprintln!("[mosaico] daemon stopped");
                return true;
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!(
                    "[mosaico] daemon shutdown requested but could not confirm exit: {error}"
                );
                return false;
            }
        }
        if Instant::now() >= deadline {
            eprintln!(
                "[mosaico] daemon shutdown requested but it did not exit within \
                 {DAEMON_SHUTDOWN_TIMEOUT:?}"
            );
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
