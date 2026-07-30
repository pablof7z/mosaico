//! Assertions over public command, process, and diagnostic evidence.

use std::time::Duration;

use cucumber::then;

use crate::world::MosaicoWorld;

#[then("the command succeeds")]
async fn command_succeeds(world: &mut MosaicoWorld) {
    let run = world.last_run();
    assert!(
        run.success(),
        "expected success, got status {:?}\nstdout:\n{}\nstderr:\n{}",
        run.status,
        run.stdout,
        run.stderr
    );
}

#[then("the command fails")]
async fn command_fails(world: &mut MosaicoWorld) {
    assert!(
        !world.last_run().success(),
        "expected command failure, but it succeeded"
    );
}

#[then(regex = r#"^the output contains "([^"]+)"$"#)]
async fn output_contains(world: &mut MosaicoWorld, expected: String) {
    let output = world.last_run().combined();
    assert!(
        output.contains(&expected),
        "expected output to contain {expected:?}\nactual:\n{output}"
    );
}

#[then("the output is valid JSON")]
async fn output_is_json(world: &mut MosaicoWorld) {
    serde_json::from_str::<serde_json::Value>(&world.last_run().stdout)
        .expect("stdout is valid JSON");
}

#[then("the hook returns successfully within its fail-open deadline")]
async fn hook_returns_fail_open(world: &mut MosaicoWorld) {
    let run = world.last_run();
    assert!(!run.timed_out, "hook exceeded the harness deadline");
    assert!(run.success(), "hook failed: {}", run.combined());
    assert!(
        run.elapsed < Duration::from_secs(3),
        "hook took {:?}, beyond its fail-open deadline",
        run.elapsed
    );
}

#[then("no daemon was spawned")]
async fn no_daemon_spawned(world: &mut MosaicoWorld) {
    assert!(
        !world.daemon_socket_exists(),
        "a hook spawned a daemon in {}",
        world.current_home().display()
    );
}

#[then("one daemon owns the backend socket")]
async fn one_daemon_owns_socket(world: &mut MosaicoWorld) {
    assert!(
        world.daemon_socket_exists(),
        "the successful command left no daemon socket in {}",
        world.current_home().display()
    );
}

#[then(regex = r#"^diagnostic "([^"]+)" is "([^"]+)"$"#)]
async fn diagnostic_state(world: &mut MosaicoWorld, name: String, status: String) {
    let report: serde_json::Value =
        serde_json::from_str(&world.last_run().stdout).expect("doctor stdout is JSON");
    let checks = report["checks"].as_array().expect("doctor checks array");
    let check = checks
        .iter()
        .find(|check| check["name"] == name)
        .unwrap_or_else(|| panic!("doctor report has no {name:?} check"));
    assert_eq!(
        check["status"].as_str(),
        Some(status.as_str()),
        "unexpected status for diagnostic {name:?}: {check}"
    );
}

#[then(regex = r#"^the relay holds the root channel for "([^"]+)"$"#)]
async fn relay_holds_root(world: &mut MosaicoWorld, workspace: String) {
    assert!(
        world.relay_holds_root(&workspace),
        "relay {} never exposed kind:39000 for {workspace:?}",
        world.relay_url()
    );
}

#[then(regex = r#"^"([^"]+)" shows workspace "([^"]+)"$"#)]
async fn backend_shows_workspace(world: &mut MosaicoWorld, backend: String, workspace: String) {
    assert!(
        world.wait_until_backend_lists(&backend, &workspace),
        "backend {backend:?} did not show workspace {workspace:?}\n{}",
        world.last_run().combined()
    );
}

#[then("no filesystem state is shared between the backends")]
async fn no_shared_filesystem(world: &mut MosaicoWorld) {
    assert!(
        world.backends_are_filesystem_isolated(&["laptop", "server"]),
        "backend roots overlap"
    );
}

#[then("the Claude process receives exactly the bundle arguments and profile selector")]
async fn exact_claude_argv(world: &mut MosaicoWorld) {
    assert_eq!(
        world.harness_argv(),
        ["--dangerously-skip-permissions", "--agent", "reviewer",],
        "the admitted runtime must assemble one exact native argv"
    );
}

#[then("no selector belonging to another harness is present")]
async fn no_foreign_selector(world: &mut MosaicoWorld) {
    let argv = world.harness_argv();
    for foreign in ["--profile", "--resume", "--session", "--agent-file"] {
        assert!(
            !argv.iter().any(|argument| argument == foreign),
            "foreign selector {foreign:?} leaked into Claude argv {argv:?}"
        );
    }
}

#[then("no legacy terminal multiplexer was invoked")]
async fn no_legacy_terminal_host(world: &mut MosaicoWorld) {
    assert!(
        world.legacy_terminal_host_was_not_invoked(),
        "the portable PTY launch attempted to invoke the forbidden legacy host"
    );
}

#[then(regex = r#"^the roster resolves that peer as "([^"]+)" without an explicit lookup$"#)]
async fn roster_resolves_peer(world: &mut MosaicoWorld, name: String) {
    assert!(
        world.wait_until_roster_names_peer(&name),
        "the relay-only peer never resolved as @{name}\n{}",
        world.last_run().combined()
    );
}

#[then("the backend management identity is absent from the member roster")]
async fn management_identity_absent(world: &mut MosaicoWorld) {
    assert!(
        world.management_identity_is_absent_from_roster(),
        "the backend management identity leaked into the roster\n{}",
        world.last_run().combined()
    );
}

#[then(regex = r#"^the native harness receives "([^"]+)" exactly once$"#)]
async fn harness_receives_once(world: &mut MosaicoWorld, body: String) {
    assert!(
        world.wait_until_harness_receives_once(&body),
        "native harness input did not contain exactly one {body:?}"
    );
}

#[then(regex = r#"^the relay records a management reply containing "([^"]+)"$"#)]
async fn management_reply(world: &mut MosaicoWorld, expected: String) {
    assert!(
        world.wait_for_management_reply(&expected),
        "no relay-visible management reply contained {expected:?}"
    );
}

#[then(regex = r#"^agent "([^"]+)" is live under the same public identity with no sibling$"#)]
async fn same_identity_no_sibling(world: &mut MosaicoWorld, agent: String) {
    assert!(
        world.same_session_is_live_without_sibling(&agent),
        "agent {agent:?} did not recover under exactly one unchanged public identity\n{}",
        world.last_run().combined()
    );
}

#[then(regex = r#"^the relay message "([^"]+)" is authored by the explicitly selected session$"#)]
async fn explicit_session_authored_message(world: &mut MosaicoWorld, body: String) {
    assert!(
        world.relay_message_was_authored_by_explicit_session(&body),
        "relay never exposed {body:?} under the explicitly selected signer"
    );
}
