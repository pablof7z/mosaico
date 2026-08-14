# Codex integration

Install Mosaico's current Codex lifecycle hooks with:

```bash
mosaico setup --harness codex
```

The installer owns Mosaico hook groups under `~/.codex/hooks.json`, including
`PreToolUse` for cooperative cross-project guidance. It preserves unrelated
hook groups. Run `mosaico setup --status` to inspect the installation; rerun
setup with the same explicit harness selection to repair it.
