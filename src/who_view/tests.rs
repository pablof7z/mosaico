use super::{render_agent_who, AgentWhoInput};
use crate::state::{AgentRoster, Status, Store};

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
            "codex-quill-peak-369",
            "codex-quill-peak-369",
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
            slug: "codex-quill-peak-369".into(),
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

fn render(all_workspaces: bool) -> String {
    let store = seed();
    let roots = vec!["alpha".to_string(), "beta".to_string()];
    render_agent_who(
        &store,
        AgentWhoInput {
            roots: &roots,
            current_root: "alpha",
            self_name: "codex-quill-peak-369",
            self_pubkey: "self-pk",
            local_host: "laptop",
            backend_pubkey: "backend-pk",
            now: 100,
            all_workspaces,
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
        "<workspace name=\"beta\" channel=\"beta\" about=\"Beta workspace\" members=\"0\" />"
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
        "<agent name=\"@codex-quill-peak-369\" state=\"idle\" status=\"Implement awareness\""
    ));
    assert!(xml.contains("members=\"2\">\n      <members>"), "{xml}");
    assert!(xml.contains(
        "<channel name=\"small-talk\" id=\"alpha.small-talk\" members=\"1\" about=\"Chit chat\" />"
    ));
    assert!(!xml.contains("general"), "synthetic root leaked: {xml}");
    assert!(!xml.contains("@remote\" state="), "backend leaked: {xml}");
}

#[test]
fn all_workspaces_expands_workspace_but_not_unjoined_root_membership() {
    let xml = render(true);
    assert!(xml.contains(
        "<workspace name=\"beta\" channel=\"beta\" about=\"Beta workspace\" members=\"0\" />"
    ));
}
