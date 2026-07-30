//! Scenario topology and installed-state fixtures.

use cucumber::given;

use crate::world::MosaicoWorld;

#[given("an isolated configured Mosaico home using a local relay")]
async fn isolated_with_nak(world: &mut MosaicoWorld) {
    world.isolated_with_nak();
}

#[given("an isolated configured Mosaico home using a fresh NIP-29 relay")]
async fn isolated_with_croissant(world: &mut MosaicoWorld) {
    world.isolated_with_croissant();
}

#[given("a fresh NIP-29 relay")]
async fn fresh_nip29_relay(world: &mut MosaicoWorld) {
    world.start_croissant();
}

#[given(regex = r#"^backends "([^"]+)" and "([^"]+)" have isolated homes$"#)]
async fn isolated_backends(world: &mut MosaicoWorld, first: String, second: String) {
    world.add_isolated_backend(&first);
    world.add_isolated_backend(&second);
}

#[given("both backends trust the same operator")]
async fn shared_operator(world: &mut MosaicoWorld) {
    world.trust_shared_operator(&["laptop", "server"]);
}

#[given("no Mosaico daemon is running")]
async fn no_daemon(world: &mut MosaicoWorld) {
    assert!(
        !world.daemon_socket_exists(),
        "the isolated world unexpectedly has a daemon socket"
    );
}

#[given(regex = r#"^the backend starts an agent in workspace "([^"]+)"$"#)]
async fn backend_starts_agent(world: &mut MosaicoWorld, workspace: String) {
    world.start_agent_in_workspace("local", &workspace);
    world.keep_workspace_observation_live(&workspace);
}

#[given(regex = r#"^Claude agent "([^"]+)" is live in workspace "([^"]+)"$"#)]
async fn live_claude_agent(world: &mut MosaicoWorld, agent: String, workspace: String) {
    world.configure_claude_agent_in_workspace(&agent, &agent, &workspace);
}

#[given(
    regex = r#"^stable Claude agent "([^"]+)" is configured but offline in workspace "([^"]+)"$"#
)]
async fn stable_claude_agent(world: &mut MosaicoWorld, agent: String, workspace: String) {
    world.configure_stable_claude_agent(&agent, &workspace);
}

#[given(regex = r#"^Claude agents "([^"]+)" and "([^"]+)" are live in workspace "([^"]+)"$"#)]
async fn two_live_claude_agents(
    world: &mut MosaicoWorld,
    first: String,
    second: String,
    workspace: String,
) {
    world.configure_two_claude_agents(&first, &second, &workspace);
}
