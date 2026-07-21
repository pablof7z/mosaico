use super::Home;
use std::os::unix::fs::PermissionsExt as _;

pub(crate) fn install_test_harness_shim(home: &std::path::Path) {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let body = r#"#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  echo 1.43.0
  exit 0
fi
case "${1:-forever}" in
  acp)
    test -n "${MOSAICO_PUBKEY:-}" && test -n "${AGENT_NSEC:-}" || exit 3
    umask 077
    printf '%s\n%s\n' "$MOSAICO_PUBKEY" "$AGENT_NSEC" > "$MOSAICO_HOME/captured-acp-identity"
    while IFS= read -r line; do
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
      test -n "$id" || continue
      case "$line" in
        *'"method":"session/new"'*) result='{"sessionId":"test-native-session"}' ;;
        *'"method":"session/prompt"'*)
          printf '%s\n' "$line" >> "$MOSAICO_HOME/captured-acp-prompts"
          if [ -n "${GOOSE_MOIM_MESSAGE_FILE:-}" ]; then
            printf '{"session_id":"test-native-session","working_dir":"%s","pid":%s}\n' "$PWD" "$$" \
              | "$MOSAICO_BIN" harness hook goose --type user-prompt-submit \
              >> "$MOSAICO_HOME/captured-goose-hook" 2>&1
            cat "$GOOSE_MOIM_MESSAGE_FILE" > "$MOSAICO_HOME/captured-goose-context"
          fi
          result='{"stopReason":"end_turn"}'
          ;;
        *) result='{}' ;;
      esac
      printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
    done
    ;;
  capture-identity)
    test -n "${MOSAICO_PUBKEY:-}" && test -n "${AGENT_NSEC:-}" || exit 3
    umask 077
    printf '%s\n%s\n' "$MOSAICO_PUBKEY" "$AGENT_NSEC" > "$2"
    while :; do sleep 1; done
    ;;
  sleep-2) sleep 2 ;;
  exit-0) exit 0 ;;
  exit-1) exit 1 ;;
  forever) while :; do sleep 1; done ;;
  *) exit 2 ;;
esac
"#;
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
