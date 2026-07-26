use super::*;

#[test]
fn pane_title_uses_session_workspace_and_joined_channels() {
    let pane = SessionPane {
        short: "6a4ddbe6".into(),
        root: "aaa".into(),
        agent: "pearl-cliff-395-haiku".into(),
        channels: vec!["aaa".into(), "dev".into()],
        ..SessionPane::default()
    };

    let title = pane_title(&pane)
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(title, "pearl-cliff-395-haiku / aaa / aaa, dev");
}
