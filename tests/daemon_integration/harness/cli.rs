use super::*;

/// Run the real `mosaico` binary as a subprocess with the home's env — i.e.
/// exactly how the hooks invoke it through the synchronous daemon client.
pub(crate) fn run_cli(home: &Home, args: &[&str]) -> std::process::Output {
    cli_command(home, args).output().expect("run mosaico")
}

pub(crate) fn run_cli_with_env(
    home: &Home,
    args: &[&str],
    env: &[(&str, &str)],
) -> std::process::Output {
    let mut cmd = cli_command(home, args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("run mosaico")
}

pub(crate) fn run_cli_with_env_in_dir(
    home: &Home,
    args: &[&str],
    env: &[(&str, &str)],
    cwd: &std::path::Path,
) -> std::process::Output {
    let mut cmd = cli_command(home, args);
    cmd.current_dir(cwd);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("run mosaico")
}

fn cli_command(home: &Home, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new(bin());
    cmd.args(args)
        // Isolate from the invoking shell's mosaico env (a live claude/codex
        // shell exports these), so session pubkey derivation is deterministic.
        .env_remove("MOSAICO_AGENT")
        .env_remove("MOSAICO_PUBKEY")
        .env_remove("MOSAICO_PTY_SESSION")
        .env_remove("MOSAICO_PTY_SOCKET")
        .env_remove("MOSAICO_CHANNEL")
        .env_remove("MOSAICO_EPHEMERAL")
        .env_remove("MOSAICO")
        .env("MOSAICO_HOME", home.dir.path())
        .env("MOSAICO_CONFIG", home.dir.path().join("config.json"))
        .env("MOSAICO_BIN", bin())
        .env("MOSAICO_DAEMON_GRACE_S", "30");
    cmd
}

// Like run_cli, but pipes `stdin` to the child — used to drive the `hook`
// subcommand, which reads its harness payload from stdin (there are no longer
// any session/turn subcommands to call directly).
pub(crate) fn run_cli_stdin(home: &Home, args: &[&str], stdin: &str) -> std::process::Output {
    run_cli_stdin_with_env(home, args, stdin, &[])
}

pub(crate) fn run_cli_stdin_with_env(
    home: &Home,
    args: &[&str],
    stdin: &str,
    env: &[(&str, &str)],
) -> std::process::Output {
    use std::io::Write as _;
    let mut cmd = cli_command(home, args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn mosaico");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("run mosaico")
}

pub(crate) fn run_cli_stdin_with_env_in_dir(
    home: &Home,
    args: &[&str],
    stdin: &str,
    env: &[(&str, &str)],
    cwd: &std::path::Path,
) -> std::process::Output {
    use std::io::Write as _;
    let mut cmd = cli_command(home, args);
    cmd.current_dir(cwd);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn mosaico");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("run mosaico")
}
