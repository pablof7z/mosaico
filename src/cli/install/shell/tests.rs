use super::*;
use std::path::PathBuf;

fn harness(id: &'static str) -> Harness {
    Harness {
        id,
        display: id,
        config_path: PathBuf::from("/tmp/unused"),
        detected: true,
    }
}

#[test]
fn managed_block_preserves_foreign_profile_content() {
    let content = "export EDITOR=vim\nalias codex=\"codex --yolo\"\n";
    let lines = vec!["alias codex=\"mosaico codex --\"".to_string()];

    let installed = rewrite_block(content, &lines).unwrap();
    assert!(installed.starts_with(content));
    assert!(installed.contains(BLOCK_START));
    assert!(installed.contains(&lines[0]));
    assert!(installed.contains("alias codex=\"codex --yolo\""));

    let removed = rewrite_block(&installed, &[]).unwrap();
    assert_eq!(removed, content);
}

#[test]
fn replacing_wrapper_selection_is_idempotent() {
    let codex = harness("codex");
    let claude = harness("claude-code");
    let all = vec![claude, codex];
    let selected = HashSet::from(["codex"]);
    let lines = wrapper_lines(&all, &selected, Syntax::Posix);

    let first = rewrite_block("# user config\n", &lines).unwrap();
    let second = rewrite_block(&first, &lines).unwrap();

    assert_eq!(first, second);
    assert_eq!(managed_ids(&second, &all, Syntax::Posix).unwrap(), selected);
}

#[test]
fn removing_one_wrapper_keeps_the_others() {
    let all = vec![harness("claude-code"), harness("codex")];
    let both = rewrite_block(
        "# user config\n",
        &wrapper_lines(
            &all,
            &HashSet::from(["claude-code", "codex"]),
            Syntax::Posix,
        ),
    )
    .unwrap();

    let kept = managed_ids(&both, &all, Syntax::Posix)
        .unwrap()
        .difference(&HashSet::from(["codex"]))
        .copied()
        .collect::<HashSet<_>>();
    let scoped = rewrite_block(&both, &wrapper_lines(&all, &kept, Syntax::Posix)).unwrap();

    assert_eq!(
        managed_ids(&scoped, &all, Syntax::Posix).unwrap(),
        HashSet::from(["claude-code"])
    );
    assert!(!scoped.contains("alias codex="));
    assert!(scoped.contains("# user config"));
}

#[test]
fn malformed_or_inline_markers_are_rejected() {
    assert!(rewrite_block(BLOCK_START, &[]).is_err());
    assert!(rewrite_block(&format!("prefix {BLOCK_START}\n{BLOCK_END}\n"), &[]).is_err());
}

#[test]
fn fish_uses_its_own_alias_syntax() {
    assert_eq!(
        wrapper_line(&harness("codex"), Syntax::Fish),
        "alias codex \"mosaico codex --\""
    );
}
