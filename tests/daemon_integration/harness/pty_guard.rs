use super::wait_until;
use anyhow::{bail, Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::time::{Duration, Instant};

/// Failure-safe owner for one exact test-spawned PTY supervisor and child.
pub(crate) struct PtyProcessGuard {
    endpoint_id: String,
    instance_token: String,
    supervisor_pid: i32,
    child_pid: i32,
    armed: bool,
}

impl PtyProcessGuard {
    pub(crate) fn capture(endpoint_id: &str) -> Self {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last_metadata = None;
        loop {
            if let Some(metadata) = metadata(endpoint_id) {
                let supervisor_pid = i32::try_from(metadata.supervisor_pid).ok();
                let child_pid = metadata.child_pid.and_then(|pid| i32::try_from(pid).ok());
                if let (Some(supervisor_pid), Some(child_pid)) = (supervisor_pid, child_pid) {
                    let guard = Self {
                        endpoint_id: endpoint_id.to_string(),
                        instance_token: metadata.instance_token,
                        supervisor_pid,
                        child_pid,
                        armed: true,
                    };
                    guard.assert_exact_processes_live();
                    return guard;
                }
                last_metadata = Some(metadata);
            }
            if Instant::now() >= deadline {
                let cleanup = cleanup_incomplete_capture(endpoint_id, last_metadata.as_ref());
                panic!(
                    "hosted PTY metadata never identified its exact supervisor and child; \
                     cleanup={cleanup:?}"
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub(crate) fn assert_exact_processes_live(&self) {
        self.verify_persisted_metadata()
            .expect("PTY endpoint metadata changed");
        self.verify_supervisor_ownership()
            .expect("PTY endpoint changed supervisor ownership");
        self.verify_child_ownership()
            .expect("PTY endpoint changed child ownership");
        assert!(
            mosaico::pty::is_live(&self.endpoint_id),
            "PTY supervisor socket is not live"
        );
        assert!(
            pid_alive(self.supervisor_pid),
            "PTY supervisor is not alive"
        );
        assert!(pid_alive(self.child_pid), "PTY child is not alive");
        assert_eq!(
            process_parent(self.child_pid),
            Some(self.supervisor_pid),
            "the captured PTY child is no longer owned by the exact supervisor"
        );
    }

    pub(crate) fn finish(mut self) {
        if let Err(error) = self.stop_exact() {
            panic!("exact PTY cleanup failed: {error:#}");
        }
        self.armed = false;
    }

    fn stop_exact(&self) -> Result<()> {
        if pid_alive(self.supervisor_pid) {
            self.verify_supervisor_ownership()?;
            let _ = mosaico::pty::kill(&self.endpoint_id);
        }
        if wait_until(Duration::from_secs(5), || self.stopped()) {
            return Ok(());
        }
        for signal in [Signal::SIGTERM, Signal::SIGKILL] {
            if pid_alive(self.child_pid) {
                self.verify_child_ownership()?;
                kill(Pid::from_raw(self.child_pid), signal)
                    .with_context(|| format!("sending {signal:?} to exact PTY child"))?;
            }
            if pid_alive(self.supervisor_pid) {
                self.verify_supervisor_ownership()?;
                kill(Pid::from_raw(self.supervisor_pid), signal)
                    .with_context(|| format!("sending {signal:?} to exact PTY supervisor"))?;
            }
            if wait_until(Duration::from_secs(5), || self.stopped()) {
                return Ok(());
            }
        }
        bail!(
            "PTY endpoint {:?} survived cleanup; supervisor_pid={} child_pid={}",
            self.endpoint_id,
            self.supervisor_pid,
            self.child_pid
        )
    }

    fn verify_persisted_metadata(&self) -> Result<()> {
        let current = metadata(&self.endpoint_id).context("missing persisted PTY metadata")?;
        if i32::try_from(current.supervisor_pid).ok() != Some(self.supervisor_pid)
            || current.instance_token != self.instance_token
            || current.child_pid.and_then(|pid| i32::try_from(pid).ok()) != Some(self.child_pid)
        {
            bail!("persisted PTY ownership changed");
        }
        Ok(())
    }

    fn verify_supervisor_ownership(&self) -> Result<()> {
        let command = process_command(self.supervisor_pid)
            .context("inspect PTY supervisor command")?
            .context("PTY supervisor is not running")?;
        if !command_owns_endpoint(&command, &self.endpoint_id, &self.instance_token) {
            bail!("PTY supervisor command no longer owns the endpoint");
        }
        Ok(())
    }

    fn verify_child_ownership(&self) -> Result<()> {
        self.verify_supervisor_ownership()?;
        if process_parent(self.child_pid) != Some(self.supervisor_pid) {
            bail!("PTY child is no longer owned by the exact supervisor");
        }
        Ok(())
    }

    fn stopped(&self) -> bool {
        !pid_alive(self.supervisor_pid)
            && !pid_alive(self.child_pid)
            && !mosaico::pty::is_live(&self.endpoint_id)
    }
}

impl Drop for PtyProcessGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.stop_exact();
        }
    }
}

fn metadata(endpoint_id: &str) -> Option<mosaico::pty::LaunchMetadata> {
    mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|row| row.id == endpoint_id)
}

fn cleanup_incomplete_capture(
    endpoint_id: &str,
    metadata: Option<&mosaico::pty::LaunchMetadata>,
) -> Result<()> {
    let Some(metadata) = metadata else {
        mosaico::pty::kill(endpoint_id).context("stop PTY endpoint with missing metadata")?;
        if wait_until(Duration::from_secs(5), || {
            !mosaico::pty::is_live(endpoint_id)
        }) {
            return Ok(());
        }
        bail!("PTY endpoint with missing metadata survived cleanup");
    };
    let supervisor_pid =
        i32::try_from(metadata.supervisor_pid).context("PTY supervisor PID overflow")?;
    let child_pid = metadata
        .child_pid
        .and_then(|pid| i32::try_from(pid).ok())
        .or_else(|| discover_owned_child(supervisor_pid))
        .context("cannot identify PTY child for capture-failure cleanup")?;
    let mut guard = PtyProcessGuard {
        endpoint_id: endpoint_id.to_string(),
        instance_token: metadata.instance_token.clone(),
        supervisor_pid,
        child_pid,
        armed: true,
    };
    let result = guard.stop_exact();
    guard.armed = false;
    result
}

fn pid_alive(pid: i32) -> bool {
    pid > 0 && process_command(pid).ok().flatten().is_some()
}

fn process_parent(pid: i32) -> Option<i32> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
        .flatten()
}

fn discover_owned_child(supervisor_pid: i32) -> Option<i32> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-ax", "-o", "pid=", "-o", "ppid=", "-o", "state="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let rows = String::from_utf8_lossy(&output.stdout);
    let mut children = rows.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        let pid = fields.next()?.parse().ok()?;
        let parent = fields.next()?.parse::<i32>().ok()?;
        let state = fields.next()?;
        (parent == supervisor_pid && !state.starts_with('Z')).then_some(pid)
    });
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

fn process_command(pid: i32) -> Result<Option<String>> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "state=", "-o", "command="])
        .output()
        .context("inspect process command")?;
    if !output.status.success() {
        return Ok(None);
    }
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let Some((state, command)) = line.split_once(char::is_whitespace) else {
        return Ok(None);
    };
    if state.starts_with('Z') {
        return Ok(None);
    }
    Ok(Some(command.trim().to_string()))
}

fn command_owns_endpoint(command: &str, endpoint_id: &str, instance_token: &str) -> bool {
    let Some(argv) = shlex::split(command.trim()) else {
        return false;
    };
    if endpoint_id.is_empty()
        || instance_token.is_empty()
        || argv.get(1).map(String::as_str) != Some("__pty-supervisor")
    {
        return false;
    }
    let options = &argv[2..argv[2..]
        .iter()
        .position(|arg| arg == "--")
        .map_or(argv.len(), |offset| offset + 2)];
    exact_option(options, "--id") == Some(endpoint_id)
        && exact_option(options, "--instance-token") == Some(instance_token)
}

fn exact_option<'a>(argv: &'a [String], option: &str) -> Option<&'a str> {
    argv.windows(2)
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
}
