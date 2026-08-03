use std::time::{Duration, Instant};

use super::Backend;

impl Backend {
    pub(in crate::world) fn live_resources(&self) -> Vec<String> {
        let mut resources = Vec::new();
        if self.socket().exists() {
            resources.push(format!("daemon socket {}", self.socket().display()));
        }
        // Prefer process ownership over socket connect: a supervisor can stay
        // alive after its socket is gone (or refuse KILL) and still leak.
        for metadata in self.pty_metadata() {
            if supervisor_still_running(&metadata) {
                resources.push(format!("PTY {}", metadata.id));
            }
        }
        resources
    }

    pub(super) fn stop_pty_supervisors(&self) {
        // Point the shared reaper at this backend's home for the duration of
        // teardown. BDD backends do not share process-global MOSAICO_HOME with
        // each other during stop() — each stop() runs sequentially in Drop.
        // SAFETY: scenario teardown is single-threaded on this world.
        unsafe {
            std::env::set_var("MOSAICO_HOME", &self.mosaico_home);
        }
        match mosaico::pty::reap_home_supervisors() {
            Ok(report) => {
                for error in report.errors {
                    eprintln!("BDD PTY reap: {error}");
                }
            }
            Err(error) => eprintln!("BDD PTY reap aborted: {error:#}"),
        }
        // Soft socket KILL as a second pass for anything metadata lost.
        for metadata in self.pty_metadata() {
            let _ = mosaico::pty::kill(&metadata.id);
            if let Some(pid) = i32::try_from(metadata.supervisor_pid)
                .ok()
                .filter(|p| *p > 1)
            {
                if process_alive(pid) {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
            }
            if let Some(pid) = metadata.child_pid.and_then(|p| i32::try_from(p).ok()) {
                if process_alive(pid) {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self
                .pty_metadata()
                .into_iter()
                .all(|metadata| !supervisor_still_running(&metadata))
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let leftover = self
            .pty_metadata()
            .into_iter()
            .filter(supervisor_still_running)
            .map(|metadata| metadata.id)
            .collect::<Vec<_>>();
        if !leftover.is_empty() {
            eprintln!(
                "BDD teardown could not stop PTY supervisors: {}",
                leftover.join(", ")
            );
        }
    }

    fn pty_metadata(&self) -> Vec<mosaico::pty::LaunchMetadata> {
        let directory = self.mosaico_home.join("pty");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect()
    }

    pub(super) fn stop_daemon(&self) {
        if !self.socket().exists() {
            return;
        }
        let _ = self.run(&["daemon", "stop"], None, Duration::from_secs(10));
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.socket().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        force_kill_daemons_for_home(&self.mosaico_home);
    }
}

fn supervisor_still_running(metadata: &mosaico::pty::LaunchMetadata) -> bool {
    let Ok(pid) = i32::try_from(metadata.supervisor_pid) else {
        return false;
    };
    pid > 1 && process_alive(pid)
}

fn process_alive(pid: i32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
        && std::fs::read(format!("/proc/{pid}/cmdline"))
            .ok()
            .is_some_and(|bytes| !bytes.is_empty())
}

fn force_kill_daemons_for_home(home: &std::path::Path) {
    let needle = format!("MOSAICO_HOME={}", home.display());
    let Ok(output) = std::process::Command::new("/bin/ps")
        .args(["-eo", "pid=", "-o", "args="])
        .output()
    else {
        return;
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        let Some((pid, args)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid.parse::<i32>() else {
            continue;
        };
        let tokens: Vec<&str> = args.split_whitespace().collect();
        let is_daemon = tokens.iter().any(|arg| arg.ends_with("mosaico"))
            && tokens.contains(&"daemon")
            && !tokens.contains(&"__pty-supervisor");
        if !is_daemon {
            continue;
        }
        let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else {
            continue;
        };
        if String::from_utf8_lossy(&env)
            .split('\0')
            .any(|entry| entry == needle)
        {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}
