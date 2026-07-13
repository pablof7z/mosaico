use super::{render_agent_who, AgentWhoInput};
use crate::state::{AgentRoster, Status, Store};
use std::collections::BTreeSet;

fn seed() -> Store {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("alpha", "alpha", "Alpha workspace", "", 1)
        .unwrap();
    store
        .upsert_channel("beta", "beta", "Beta workspace", "", 1)
        .unwrap();
    store
        .upsert_channel("joined", "planning", "Plan work", "alpha", 1)
        .unwrap();
    store
        .upsert_channel("compact", "small-talk", "Chit chat", "alpha", 1)
        .unwrap();
    store.upsert_workspace("alpha", "/work/alpha", 1).unwrap();
    store
        .upsert_profile_with_agent_slug(
            "self-pk",
            "quill-peak-369-codex",
            "quill-peak-369-codex",
            "codex",
            "laptop",
            false,
            1,
        )
        .unwrap();
    store
        .upsert_profile("human-pk", "Pablo", "Pablo", "", false, 1)
        .unwrap();
    store
        .upsert_profile("backend-pk", "remote", "remote", "remote", true, 1)
        .unwrap();
    for channel in ["alpha", "joined"] {
        store
            .upsert_channel_member(channel, "self-pk", "member", 1)
            .unwrap();
    }
    store
        .upsert_channel_member("beta", "self-pk", "member", 1)
        .unwrap();
    store
        .upsert_channel_member("alpha", "human-pk", "admin", 1)
        .unwrap();
    store
        .upsert_channel_member("alpha", "backend-pk", "admin", 1)
        .unwrap();
    store
        .upsert_channel_member("compact", "human-pk", "member", 1)
        .unwrap();
    store
        .upsert_status(&Status {
            pubkey: "self-pk".into(),
            session_id: "session".into(),
            channel_h: "alpha".into(),
            slug: "quill-peak-369-codex".into(),
            title: "Implement awareness".into(),
            activity: String::new(),
            busy: false,
            last_seen: 99,
            updated_at: 99,
            expiration: 200,
        })
        .unwrap();
    store
        .replace_agent_roster(&AgentRoster {
            backend_pubkey: "remote-pk".into(),
            host: "remoteBackend1".into(),
            slug: "claude".into(),
            use_criteria: "Remote review".into(),
            channels: vec!["joined".into(), "beta".into()],
            updated_at: 1,
        })
        .unwrap();
    store
        .upsert_channel_member("alpha", "remote-pk", "admin", 1)
        .unwrap();
    store
}

fn render(expand_beta: bool) -> String {
    let store = seed();
    let roots = vec!["alpha".to_string(), "beta".to_string()];
    let mut expanded_workspaces = BTreeSet::from(["alpha".to_string()]);
    if expand_beta {
        expanded_workspaces.insert("beta".to_string());
    }
    render_agent_who(
        &store,
        AgentWhoInput {
            roots: &roots,
            self_name: "quill-peak-369-codex",
            self_pubkey: "self-pk",
            local_host: "laptop",
            backend_pubkey: "backend-pk",
            now: 100,
            expanded_workspaces: &expanded_workspaces,
        },
    )
}

#[test]
fn lists_global_agents_and_compacts_other_workspaces() {
    let xml = render(false);
    assert!(
        xml.contains("<agent name=\"claude@remoteBackend1\""),
        "{xml}"
    );
    assert!(
        xml.contains("workspace-availability=\"alpha,beta\""),
        "{xml}"
    );
    assert!(
        xml.contains("<workspace name=\"alpha\" channel=\"alpha\" path=\"/work/alpha\""),
        "{xml}"
    );
    assert!(xml.contains(
        "<workspace name=\"beta\" channel=\"beta\" about=\"Beta workspace\" members=\"1\" />"
    ));
}

#[test]
fn workspace_carries_root_members_and_membership_gated_children() {
    let xml = render(false);
    assert!(
        xml.contains(
            "<workspace name=\"alpha\" channel=\"alpha\" path=\"/work/alpha\" about=\"Alpha workspace\" members=\"2\""
        ),
        "{xml}"
    );
    assert!(xml.contains("<human name=\"@Pablo\" state=\"offline\""));
    assert!(xml.contains(
        "<agent name=\"@quill-peak-369-codex\" state=\"idle\" status=\"Implement awareness\""
    ));
    assert!(xml.contains("members=\"2\">\n      <members>"), "{xml}");
    assert!(xml.contains(
        "<channel name=\"small-talk\" id=\"alpha.small-talk\" members=\"1\" about=\"Chit chat\" />"
    ));
    assert!(!xml.contains("general"), "synthetic root leaked: {xml}");
    assert!(!xml.contains("@remote\" state="), "backend leaked: {xml}");
}

#[test]
fn exact_session_joined_workspace_set_controls_expansion() {
    let xml = render(true);
    assert!(xml.contains(
        "<workspace name=\"beta\" channel=\"beta\" about=\"Beta workspace\" members=\"1\">"
    ));
    assert!(xml.contains("<agent name=\"@quill-peak-369-codex\" state=\"offline\""));
}
