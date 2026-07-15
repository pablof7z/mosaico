use super::Home;
use std::os::unix::fs::PermissionsExt as _;

pub(crate) fn install_test_harness_shim(home: &std::path::Path) {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = bin.join("opencode");
    std::fs::write(
        &shim,
        "#!/bin/sh\ncase \"${1:-forever}\" in\n  sleep-2) sleep 2 ;;\n  exit-0) exit 0 ;;\n  exit-1) exit 1 ;;\n  forever) while :; do sleep 1; done ;;\n  *) exit 2 ;;\nesac\n",
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        std::env::var_os("PATH").as_deref().unwrap_or_default(),
    ));
    unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };
}

pub(crate) fn configure_pty_agent(home: &Home, slug: &str, mode: &str) {
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        serde_json::json!({
            "test-pty": {
                "harness": "opencode",
                "transport": "pty",
                "args": [mode],
            }
        })
        .to_string(),
    )
    .unwrap();
    mosaico::identity::add_local_agent(home.dir.path(), slug, "test-pty", None, 1).unwrap();
}
