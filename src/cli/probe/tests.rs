use super::*;

#[test]
fn unimplemented_shape_renders_marker() {
    let v = json!({ "verb": "simulate", "implemented": false,
                    "message": "subscriptions simulate is a v2 follow-up" });
    let text = render("simulate", &v);
    assert_eq!(
        text,
        "probe simulate: subscriptions simulate is a v2 follow-up\n"
    );
}

#[test]
fn stats_action_projects_rpc_params() {
    let action = ProbeAction::Stats {
        surface: Some("status".into()),
        since: 42,
    };
    let (verb, params) = action.to_rpc().unwrap();
    assert_eq!(verb, "stats");
    assert_eq!(params["verb"], "stats");
    assert_eq!(params["surface"], "status");
    assert_eq!(params["since"], 42);
}

#[test]
fn seams_action_projects_rpc_params() {
    let (verb, params) = ProbeAction::Seams.to_rpc().unwrap();
    assert_eq!(verb, "seams");
    assert_eq!(params["verb"], "seams");
}

#[test]
fn replay_action_projects_rpc_params() {
    let action = ProbeAction::Replay {
        capsule: "42".into(),
        assert_replay: true,
        export_trace: Some(PathBuf::from("trace.json")),
    };
    let (verb, params) = action.to_rpc().unwrap();
    assert_eq!(verb, "replay");
    assert_eq!(params["capsule"], "42");
    assert_eq!(params["assert"], true);
    assert_eq!(params["export_trace"], true);
}

#[test]
fn simulate_action_projects_rpc_params() {
    let action = ProbeAction::Simulate {
        surface: "status".into(),
        fact: None,
        session: Some("s1".into()),
        activity: Some("reviewing the PR".into()),
        title: None,
        now: None,
    };
    let (verb, params) = action.to_rpc().unwrap();
    assert_eq!(verb, "simulate");
    assert_eq!(params["session"], "s1");
    assert_eq!(params["activity"], "reviewing the PR");
    assert!(params["title"].is_null());
}

#[test]
fn simulate_action_parses_fact_json() {
    let action = ProbeAction::Simulate {
        surface: "subscriptions".into(),
        fact: Some(r#"{"SubscriptionSync":{"snapshot":{"daemon_channels":[],"addressed_pubkeys":[],"archived_channels":[],"sessions":{}},"at":1}}"#.into()),
        session: None,
        activity: None,
        title: None,
        now: None,
    };
    let (_verb, params) = action.to_rpc().unwrap();
    assert!(params["fact"].is_object());
    assert_eq!(params["fact"]["SubscriptionSync"]["at"], 1);
}

#[test]
fn diff_action_projects_rpc_params() {
    let action = ProbeAction::Diff {
        surface: "status".into(),
        fact: r#"{"StatusDrive":{"Tick":{"pubkey":"s1","at":1}}}"#.into(),
        capsule: Some("7".into()),
        mutate_fact: None,
    };
    let (verb, params) = action.to_rpc().unwrap();
    assert_eq!(verb, "diff");
    assert_eq!(params["capsule"], "7");
    assert_eq!(params["fact"]["StatusDrive"]["Tick"]["pubkey"], "s1");
}

#[test]
fn acid_action_projects_rpc_params() {
    let action = ProbeAction::Acid {
        handle: "status:s1".into(),
        fact: r#"{"StatusDrive":{"Tick":{"pubkey":"s1","at":1}}}"#.into(),
        cause: Some("status/s1/activity".into()),
    };
    let (verb, params) = action.to_rpc().unwrap();
    assert_eq!(verb, "acid");
    assert_eq!(params["handle"], "status:s1");
    assert_eq!(params["cause"], "status/s1/activity");
}
