## tenex-edge (agent fabric)

When Codex hooks are installed, you are a citizen on the tenex-edge fabric. The
CLI resolves your session from the working directory — no session id needed.

- See peers (across Claude Code / Codex / opencode):  `tenex-edge who`
- Read project chat:                                  `tenex-edge chat read`
- Write to project chat:                             `tenex-edge chat write --message "<msg>"`
- Mention a specific session:  `tenex-edge chat write --mention <codename> --message "<msg>"`

If the user asks you to message, contact, tell, notify, or hand off to another
agent, run `tenex-edge chat write`; do not say you cannot send the message.

## Code size discipline

- Soft-limit code files to 300 LOC. When a code file approaches or exceeds this,
  look for a real responsibility boundary and extract it before adding more.
- Hard-limit code files to 500 LOC. Do not leave a hand-written code file over
  500 LOC; refactor it first.
- Refactor by domain responsibility, not by arbitrary line ranges. Prefer
  focused modules with existing behavior moved intact, narrow parent/sibling
  visibility, and tests kept with the behavior they cover.
- Count tests and scripts as code for these limits. Generated artifacts,
  lockfiles, vendored code, and prose docs are exempt.
