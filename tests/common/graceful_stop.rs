//! Stop an external relay child so it can close its store before a SIGKILL.

use std::process::Child;
use std::time::{Duration, Instant};

/// Croissant bounds its own HTTP drain at 5s; allow a margin on top.
const GRACE: Duration = Duration::from_secs(10);

/// SIGTERM `child`, wait up to [`GRACE`] for it to exit, then SIGKILL.
///
/// A SIGKILLed Croissant never closes LMDB. On macOS LMDB unlinks its POSIX
/// named semaphores only on a clean close, so every killed relay leaks them
/// until `kern.posix.sem.max` is exhausted and Croissant can no longer open
/// its store (mosaico #856).
pub(crate) fn stop_gracefully(child: &mut Child) {
    // Reap first: an exited, reaped child's pid may already belong to another
    // process.
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    if nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).is_ok() {
        let deadline = Instant::now() + GRACE;
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
