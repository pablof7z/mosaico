use super::Home;
use std::os::unix::fs::PermissionsExt as _;

pub(crate) fn install_test_harness_shim(home: &std::path::Path) {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let body = "#!/bin/sh\ncase \"${1:-forever}\" in\n  acp)\n    test -n \"${MOSAICO_PUBKEY:-}\" && test -n \"${AGENT_NSEC:-}\" || exit 3\n    umask 077\n    printf '%s\\n%s\\n' \"$MOSAICO_PUBKEY\" \"$AGENT_NSEC\" > \"$MOSAICO_HOME/captured-acp-identity\"\n    while IFS= read -r line; do\n      id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\\([0-9][0-9]*\\).*/\\1/p')\n      test -n \"$id\" || continue\n      case \"$line\" in\n        *'\"method\":\"session/new\"'*) result='{\"sessionId\":\"test-native-session\"}' ;;\n        *'\"method\":\"session/prompt\"'*)\n          printf '%s\\n' \"$line\" >> \"$MOSAICO_HOME/captured-acp-prompts\"\n          result='{\"stopReason\":\"end_turn\"}'\n          ;;\n        *) result='{}' ;;\n      esac\n      printf '{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":%s}\\n' \"$id\" \"$result\"\n    done\n    ;;\n  capture-identity)\n    test -n \"${MOSAICO_PUBKEY:-}\" && test -n \"${AGENT_NSEC:-}\" || exit 3\n    umask 077\n    printf '%s\\n%s\\n' \"$MOSAICO_PUBKEY\" \"$AGENT_NSEC\" > \"$2\"\n    while :; do sleep 1; done\n    ;;\n  sleep-2) sleep 2 ;;\n  exit-0) exit 0 ;;\n  exit-1) exit 1 ;;\n  forever) while :; do sleep 1; done ;;\n  *) exit 2 ;;\nesac\n";
    for executable in ["opencode", "goose"] {
        let shim = bin.join(executable);
        std::fs::write(&shim, body).unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        std::env::var_os("PATH").as_deref().unwrap_or_default(),
    ));
    unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };
}

pub(crate) fn configure_pty_agent(home: &Home, slug: &str, mode: &str) {
    configure_pty_agent_with_args(home, slug, &[mode]);
}

pub(crate) fn configure_pty_agent_with_args(home: &Home, slug: &str, args: &[&str]) {
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        serde_json::json!({
            "test-pty": {
                "harness": "opencode",
                "transport": "pty",
                "args": args,
            }
        })
        .to_string(),
    )
    .unwrap();
    mosaico::identity::add_local_agent(home.dir.path(), slug, "test-pty", None, 1).unwrap();
}
