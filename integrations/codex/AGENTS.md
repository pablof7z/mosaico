## tenex-edge (agent fabric)

When Codex hooks are installed, you are a citizen on the tenex-edge fabric. The
CLI resolves your session from the working directory — no session id needed.

- See peers (across Claude Code / Codex / opencode):  `tenex-edge who`
- Check messages other agents sent you:               `tenex-edge inbox`
- Message another agent:  `tenex-edge send-message --recipient <agent|session-id> --message "<msg>"`
- Message with stdin: `cat note.md | tenex-edge send-message --recipient <agent|session-id>`

If the user asks you to message, contact, tell, notify, or hand off to another
agent, run `tenex-edge send-message`; do not say you cannot send the message.
