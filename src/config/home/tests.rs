use super::*;

fn os(value: &str) -> Option<OsString> {
    Some(OsString::from(value))
}

#[test]
fn unset_selector_uses_the_default_home() {
    let selected = select_mosaico_home(None, None, None, os("/home/alice")).unwrap();
    assert_eq!(selected.instance, "default");
    assert_eq!(selected.mosaico_home, PathBuf::from("/home/alice/.mosaico"));
    assert!(selected.mosaico_home_is_default);
}

#[test]
fn named_selector_uses_a_sibling_instance_root() {
    let selected = select_mosaico_home(os("alternative1"), None, None, os("/home/alice")).unwrap();
    assert_eq!(selected.instance, "alternative1");
    assert_eq!(
        selected.mosaico_home,
        PathBuf::from("/home/alice/.mosaico-instances/alternative1")
    );
    assert!(!selected.mosaico_home_set);
    assert!(!selected.mosaico_home_is_default);
}

#[test]
fn explicit_default_selector_uses_the_existing_default_home() {
    let selected = select_mosaico_home(os("default"), None, None, os("/home/alice")).unwrap();
    assert_eq!(selected.mosaico_home, PathBuf::from("/home/alice/.mosaico"));
    assert!(selected.mosaico_home_is_default);
}

#[test]
fn exact_home_override_remains_available_without_a_selector() {
    let selected =
        select_mosaico_home(None, os("/tmp/mosaico-test"), None, os("/home/alice")).unwrap();
    assert_eq!(selected.mosaico_home, PathBuf::from("/tmp/mosaico-test"));
    assert!(selected.mosaico_home_set);
    assert!(!selected.mosaico_home_is_default);
}

#[test]
fn config_path_honors_exact_override_without_a_selector() {
    let selected = select_mosaico_home(None, None, None, os("/home/alice")).unwrap();
    assert_eq!(
        select_config_path(os("/tmp/config.json"), selected),
        PathBuf::from("/tmp/config.json")
    );
}

#[test]
fn selector_conflicts_with_low_level_path_overrides() {
    assert_eq!(
        select_mosaico_home(os("one"), os("/tmp/one"), None, os("/home/alice")),
        Err("MOSAICO cannot be combined with MOSAICO_HOME".into())
    );
    assert_eq!(
        select_mosaico_home(os("one"), None, os("/tmp/one.json"), os("/home/alice")),
        Err("MOSAICO cannot be combined with MOSAICO_CONFIG".into())
    );
}

#[test]
fn selector_requires_home() {
    assert_eq!(
        select_mosaico_home(os("one"), None, None, None),
        Err("HOME must be set when MOSAICO selects an instance".into())
    );
}

#[test]
fn selector_rejects_a_relative_home() {
    assert_eq!(
        select_mosaico_home(os("one"), None, None, os("relative-home")),
        Err("HOME must be an absolute path when MOSAICO selects an instance".into())
    );
}

#[test]
fn absent_home_without_an_override_is_rejected() {
    assert_eq!(
        select_mosaico_home(None, None, None, None),
        Err(MISSING_HOME_MESSAGE.into())
    );
}

#[test]
fn instance_names_are_exact_and_path_safe() {
    for accepted in ["a", "alternative1", "relay-2", "relay_3", "default"] {
        assert_eq!(
            validate_instance_name(OsStr::new(accepted)).unwrap(),
            accepted
        );
    }
    for rejected in ["", "Alternative1", "-relay", "_relay", ".", "a/b", "a b"] {
        assert!(
            validate_instance_name(OsStr::new(rejected)).is_err(),
            "accepted invalid name {rejected:?}"
        );
    }
    assert!(validate_instance_name(OsStr::new(&"a".repeat(64))).is_err());
}
