//! Public CLI and hook actions.

use std::time::Duration;

use cucumber::when;

use crate::world::MosaicoWorld;

#[when("a native session-start hook runs")]
async fn session_start_hook(world: &mut MosaicoWorld) {
    let cwd = world.current_home().display();
    let payload = format!(r#"{{"session_id":"bdd-session","cwd":"{cwd}"}}"#);
    world.run_with_stdin(
        &["harness", "hook", "claude-code", "--type", "session-start"],
        &payload,
        Duration::from_secs(3),
    );
}

#[when(regex = r#"^"([^"]+)" starts an agent in workspace "([^"]+)"$"#)]
async fn starts_agent(world: &mut MosaicoWorld, backend: String, workspace: String) {
    world.start_agent_in_workspace(&backend, &workspace);
}

#[when(regex = r#"^"([^"]+)" lists every visible workspace$"#)]
async fn lists_workspaces(world: &mut MosaicoWorld, backend: String) {
    world.list_channels_on(&backend);
}

#[when(regex = r#"^a relay-only peer named "([^"]+)" joins workspace "([^"]+)"$"#)]
async fn relay_only_peer(world: &mut MosaicoWorld, name: String, workspace: String) {
    world.add_relay_only_peer(&name, &workspace);
}

#[when(regex = r#"^the operator addresses that agent with "([^"]+)"$"#)]
async fn operator_addresses_agent(world: &mut MosaicoWorld, body: String) {
    world.address_live_agent(&body);
}

#[when(regex = r#"^the operator addresses that configured identity with "([^"]+)"$"#)]
async fn operator_addresses_configured_agent(world: &mut MosaicoWorld, body: String) {
    world.address_configured_agent(&body);
}

#[when(regex = r#"^the operator sends management command "([^"]+)"$"#)]
async fn management_command(world: &mut MosaicoWorld, body: String) {
    world.send_management_command(&body);
}

#[when("the operator stops that exact session")]
async fn stop_exact_session(world: &mut MosaicoWorld) {
    world.stop_active_session();
}

#[when(regex = r#"^I send "([^"]+)" with the second session explicitly selected$"#)]
async fn explicit_session_send(world: &mut MosaicoWorld, body: String) {
    world.send_with_explicit_session_anchor(&body);
}
