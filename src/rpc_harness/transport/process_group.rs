use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

pub(super) fn isolate(command: &mut tokio::process::Command) {
    command.process_group(0);
}

pub(super) fn signal(group: i32) -> std::io::Result<bool> {
    match kill(Pid::from_raw(-group), Some(Signal::SIGKILL)) {
        Ok(()) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(error) => Err(std::io::Error::from_raw_os_error(error as i32)),
    }
}

pub(super) async fn wait_exit(group: i32) -> std::io::Result<()> {
    for _ in 0..200 {
        if !has_live_members(group)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("timed out waiting for RPC harness process group {group} to exit"),
    ))
}

#[cfg(target_os = "linux")]
fn has_live_members(group: i32) -> std::io::Result<bool> {
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let stat = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let Some((state, process_group)) = parse_linux_stat(&stat) else {
            continue;
        };
        if process_group == group && !matches!(state, 'Z' | 'X') {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
fn parse_linux_stat(stat: &str) -> Option<(char, i32)> {
    let (_, fields) = stat.rsplit_once(") ")?;
    let mut fields = fields.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _parent = fields.next()?;
    let process_group = fields.next()?.parse().ok()?;
    Some((state, process_group))
}

#[cfg(not(target_os = "linux"))]
fn has_live_members(group: i32) -> std::io::Result<bool> {
    match kill(Pid::from_raw(-group), None) {
        Err(Errno::ESRCH) => Ok(false),
        // macOS can report EPERM while a just-killed process group is
        // transitioning through reap. It is not exit confirmation.
        Ok(()) | Err(Errno::EPERM) => Ok(true),
        Err(error) => Err(std::io::Error::from_raw_os_error(error as i32)),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::parse_linux_stat;

    #[test]
    fn parses_linux_process_group_and_state_after_parenthesized_command() {
        assert_eq!(
            parse_linux_stat("17 (codex app-server) S 1 17 17 0 -1"),
            Some(('S', 17))
        );
        assert_eq!(
            parse_linux_stat("18 (mcp child) Z 1 17 17 0 -1"),
            Some(('Z', 17))
        );
    }
}
