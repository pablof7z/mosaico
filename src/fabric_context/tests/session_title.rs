use super::*;

#[test]
fn agent_supplied_title_appears_immediately() {
    let store = seed_store();
    let mut rec = session(&store);
    rec.title = "Researching MCP improvements around resource allocation".into();
    rec.turn_count = 12;

    let visible = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("explicit context should render");
    assert!(
        visible.contains("<self name=\"@coder\" host=\"laptop\" headless=\"off\" unhosted=\"true\" workspace=\"root\" title=\"Researching MCP improvements around resource allocation\" />"),
        "got: {visible}"
    );
    assert!(
        visible.contains(
            "Read ~/.agents/skills/mosaico/references/public-work-status.md before updating it"
        ),
        "got: {visible}"
    );

    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 100, true)).unwrap();
    let reconciled = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    assert_eq!(reconciled, visible);
}

#[test]
fn missing_title_prompts_to_set_a_status() {
    let store = seed_store();
    let rec = session(&store); // default: no title set

    let visible = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("explicit context should render");
    assert!(
        visible.contains("No session status set"),
        "an agent with no title should be prompted to set one; got: {visible}"
    );
    assert!(
        visible.contains("~/.agents/skills/mosaico/references/public-work-status.md"),
        "got: {visible}"
    );
    assert!(
        !visible.contains("Current title"),
        "should not claim a title when none is set; got: {visible}"
    );
}

#[test]
fn self_branch_is_derived_from_the_recorded_workspace_checkout() {
    let store = seed_store();
    let rec = session(&store);
    let checkout = tempfile::tempdir().unwrap();
    let initialized = std::process::Command::new("git")
        .args(["init", "--quiet", "--initial-branch=feat/context"])
        .arg(checkout.path())
        .status()
        .expect("git should run");
    assert!(initialized.success());
    store
        .upsert_workspace("root", &checkout.path().to_string_lossy(), 2)
        .unwrap();

    let visible = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("explicit context should render");
    assert!(
        visible.contains("workspace=\"root\" branch=\"feat/context\""),
        "got: {visible}"
    );
}
