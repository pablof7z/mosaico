use super::harness_from_process;

#[test]
fn detects_node_entrypoint_without_matching_hook_arguments() {
    assert_eq!(
        harness_from_process(
            "/usr/local/bin/node",
            "/usr/local/bin/node /usr/local/lib/node_modules/@moonshot-ai/kimi-code/dist/main.mjs acp"
        ),
        Some("kimi")
    );
    assert_eq!(
        harness_from_process("mosaico", "mosaico harness hook kimi --type stop"),
        None
    );
}
