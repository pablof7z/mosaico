use super::*;

#[test]
fn agent_supplied_title_appears_immediately() {
    let store = seed_store();
    let mut rec = session(&store);
    rec.title = "Researching MCP improvements around resource allocation".into();

    let visible = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("explicit context should render");
    assert!(
        visible.contains("Agent: coder · Session: @coder · Backend: laptop\n  Current title: \"Researching MCP improvements around resource allocation\""),
        "got: {visible}"
    );
    assert!(
        visible.contains("[if your title drifted you can update it]"),
        "got: {visible}"
    );

    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 100, true)).unwrap();
    let reconciled = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    assert_eq!(reconciled, visible);
}
