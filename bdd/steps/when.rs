//! Public CLI and hook actions.

use std::time::Duration;

use cucumber::when;

use crate::world::MosaicoWorld;

#[when("I run Mosaico with no arguments")]
async fn bare_mosaico(world: &mut MosaicoWorld) {
    world.run(&[]);
}

#[when("I request diagnostic JSON")]
async fn doctor_json(world: &mut MosaicoWorld) {
    world.run(&["doctor", "--json"]);
}

#[when("I list every visible channel")]
async fn list_all_channels(world: &mut MosaicoWorld) {
    world.run(&["channel", "list", "--all"]);
}

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

#[when("I invoke the removed agents target")]
async fn removed_agents_target(world: &mut MosaicoWorld) {
    world.run(&["agents", "target"]);
}

#[when(regex = r#"^"([^"]+)" starts an agent in workspace "([^"]+)"$"#)]
async fn starts_agent(world: &mut MosaicoWorld, backend: String, workspace: String) {
    world.start_agent_in_workspace(&backend, &workspace);
}

#[when(regex = r#"^"([^"]+)" lists every visible workspace$"#)]
async fn lists_workspaces(world: &mut MosaicoWorld, backend: String) {
    world.list_channels_on(&backend);
}

#[when(regex = r#"^Mosaico launches agent "([^"]+)"$"#)]
async fn launches_agent(world: &mut MosaicoWorld, agent: String) {
    world.launch_agent(&agent);
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

#[when(regex = r#"^I search all cached channels for "([^"]+)"$"#)]
async fn search_cached_channels(world: &mut MosaicoWorld, text: String) {
    world.search_cached_messages(&text);
}
