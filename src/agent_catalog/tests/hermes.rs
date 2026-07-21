use super::*;
use crate::test_env::EnvGuard;

#[test]
fn discovers_real_hermes_metadata_and_native_selector() {
    let home = TempDir::new().unwrap();
    let profile = home.path().join(".hermes/profiles/reviewer");
    write(
        &profile.join("profile.yaml"),
        "description: Reviews completed changes for correctness, regressions, security, and\n  conformance with repository guidance.\ndescription_auto: false\n",
    );
    write(&profile.join(".env"), "SECRET_VALUE=not-catalog-metadata\n");

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    let discovered = catalog.resolve("reviewer", None, None).unwrap();

    assert_eq!(discovered.harness, Harness::Hermes);
    assert_eq!(discovered.path, profile);
    assert_eq!(
        discovered.use_criteria,
        "Reviews completed changes for correctness, regressions, security, and conformance with repository guidance."
    );
    assert_eq!(
        discovered.activation().unwrap(),
        NativeAgentActivation::NativeSelector {
            name: "reviewer".into()
        }
    );
}

#[test]
fn mirrors_hermes_profile_list_tolerance_and_name_rules() {
    let home = TempDir::new().unwrap();
    let profiles = home.path().join(".hermes/profiles");
    std::fs::create_dir_all(profiles.join("legacy")).unwrap();
    write(
        &profiles.join("broken/profile.yaml"),
        "description: [unterminated\n",
    );
    for ignored in ["default", "UPPER", "-leading", "has.dot"] {
        std::fs::create_dir_all(profiles.join(ignored)).unwrap();
    }
    write(&profiles.join("regular-file"), "not a profile directory");

    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();

    assert_eq!(catalog.slugs(), ["broken", "legacy"]);
    assert_eq!(
        catalog.resolve("broken", None, None).unwrap().use_criteria,
        ""
    );
    assert_eq!(
        catalog.resolve("legacy", None, None).unwrap().use_criteria,
        ""
    );
}

#[test]
fn installed_roots_honor_hermes_home() {
    let home = TempDir::new().unwrap();
    let custom = home.path().join("custom-hermes");
    let mut env = EnvGuard::set("HOME", home.path());
    env.set_var("HERMES_HOME", &custom);
    env.set_var("XDG_CONFIG_HOME", home.path().join(".config"));

    assert_eq!(
        DiscoveryRoots::installed().unwrap().hermes,
        custom.join("profiles")
    );

    let active = home.path().join(".hermes/profiles/builder");
    env.set_var("HERMES_HOME", &active);
    assert_eq!(
        DiscoveryRoots::installed().unwrap().hermes,
        home.path().join(".hermes/profiles")
    );
}

#[cfg(unix)]
#[test]
fn removal_delegates_to_the_exact_native_hermes_profile() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = TempDir::new().unwrap();
    let profiles = home.path().join(".hermes/profiles");
    let profile = profiles.join("reviewer");
    std::fs::create_dir_all(&profile).unwrap();
    let bin = home.path().join("bin");
    let executable = bin.join("hermes");
    let log = home.path().join("delete.log");
    write(
        &executable,
        "#!/bin/sh\nprintf '%s\\n' \"$HERMES_HOME\" \"$1\" \"$2\" \"$3\" \"$4\" > \"$HERMES_DELETE_LOG\"\n/bin/rmdir \"$HERMES_HOME/profiles/$3\"\n",
    );
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = EnvGuard::set("PATH", &bin);
    env.set_var("HERMES_DELETE_LOG", &log);
    let catalog = AgentCatalog::discover(&roots(home.path()), &[]).unwrap();
    let discovered = catalog.resolve("reviewer", None, None).unwrap();

    assert!(remove_native_profile(&discovered).unwrap());
    assert!(!profile.exists());
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        format!(
            "{}\nprofile\ndelete\nreviewer\n--yes\n",
            home.path().join(".hermes").display()
        )
    );
}
