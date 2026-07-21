use super::*;
use clap::Parser as _;

#[test]
fn purge_requires_a_separate_yes_flag_non_interactively() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "uninstall",
        "--purge-state",
        "--yes",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(cli.cmd, Some(crate::cli::args::Cmd::Uninstall(_))));

    let error = crate::cli::args::Cli::try_parse_from(["mosaico", "uninstall", "--yes"])
        .err()
        .expect("--yes without --purge-state should fail");
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn refuses_broad_or_relative_state_paths() {
    assert!(validate_state_home(Path::new("/")).is_err());
    assert!(validate_state_home(&std::env::temp_dir()).is_err());
    assert!(validate_state_home(Path::new(".mosaico")).is_err());
    let cwd = std::env::current_dir().unwrap();
    assert!(validate_state_home(&cwd).is_err());
}

#[test]
fn removes_only_the_exact_safe_state_directory() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("nested/.mosaico");
    let sibling = temp.path().join("nested/keep.txt");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("state.db"), "state").unwrap();
    std::fs::write(&sibling, "keep").unwrap();

    remove_state_home(&state).unwrap();

    assert!(!state.exists());
    assert_eq!(std::fs::read_to_string(sibling).unwrap(), "keep");
}
