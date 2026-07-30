# Codex integration

Install Mosaico's current Codex lifecycle hooks with:

```bash
mosaico setup --harness codex
```

The installer owns Mosaico hook groups under `~/.codex/hooks.json`, including
`PreToolUse` for cooperative cross-project guidance. It preserves unrelated
hook groups. Run `mosaico doctor --json` to inspect the installed integration
or `mosaico doctor --fix --json` when the user has asked to repair it.
