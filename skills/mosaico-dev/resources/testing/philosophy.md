# Testing philosophy

Testing is the practice of making a claim falsifiable and collecting evidence
from a witness that has authority to decide whether the claim is true.

For Mosaico, a useful test answers three questions:

1. What behavior or rule is claimed?
2. What failure would matter?
3. Which observer can distinguish success from a convincing imitation?

## Claims and witnesses

“A second backend sees the workspace” is a claim. The second backend's public
channel listing, backed by relay state and isolated filesystems, is a witness.
Inspecting the first backend's SQLite row is not an authoritative witness for
cross-backend discovery.

“The Claude profile becomes `--agent reviewer`” is a claim. A deterministic
Claude process shim that captures exact argv is a witness. Asserting that a
helper returned an enum is useful lower-level evidence, but does not prove the
spawned process received the argument.

Choose the cheapest witness that still has authority. Narrow evidence is easier
to run and diagnose. Broader evidence is justified when narrower observers can
pass while the product remains broken.

## Green means demonstrated behavior

Never optimize for a green dashboard, a test count, or a coverage percentage.
Those are measurements of the evidence machinery, not proof that the claims
matter.

A test that cannot fail for the defect it names creates false confidence. A
skipped scenario is not green evidence. A test made green by weakening the
assertion is a design regression.

The normal workflow separates authorship:

- the design/architecture agent writes the executable claim and explains the
  witness;
- an implementation agent changes production code;
- both may revise a claim only after showing that it was inaccurate,
  unobservable, or coupled to an implementation choice.

This separation protects the claim from being rewritten around the code that
happened to be produced.

## A portfolio, not a pyramid

Mosaico needs several evidence types because it crosses several authorities:

- unit tests localize rules and cover many edge cases cheaply;
- integration and contract tests prove technical seams;
- process tests prove daemon, socket, supervisor, and executable behavior;
- BDD states stable product promises in shared language;
- local relay tests prove Nostr or NIP-29 behavior deterministically;
- probes ask live systems questions that local code cannot answer;
- provider labs prove real authentication and transport compatibility;
- quality gates enforce source and repository constraints.

No tier is prestigious. Each owns a different uncertainty.

## Independence and complementary overlap

Keep two tests for the same area when they use different witnesses or answer
different diagnostic questions. A unit test may prove profile-selector mapping
while BDD proves exact process argv. The first localizes a mapping defect; the
second proves the complete supported launch path.

Remove duplication when two tests repeat the same setup, action, and witness
without adding failure localization, edge coverage, or product readability.

## Tests follow product lifetime

A test is part of the specification for behavior Mosaico currently owns. When
that behavior is intentionally removed, delete its BDD scenarios, unit and
integration tests, fixtures, step definitions, and dedicated helpers in the
same change.

Do not turn the former positive test into a negative “the old feature does not
exist” test. That preserves a dead concept in the current specification and
creates maintenance work for behavior Mosaico no longer owns. Git history is
the record that it once existed.

Keep a nearby test only when it independently specifies current behavior. Its
name and claim must describe that current rule without referring to the removed
feature.

## Strong defaults

- Test observable behavior rather than call order.
- Use real Mosaico components until a boundary becomes slow, unsafe, variable,
  or outside repository ownership.
- Replace external variability with pinned fixtures or deterministic shims.
- Keep every test independent of execution order.
- Bound all waits by observable evidence.
- Preserve detailed artifacts on failure without leaking secrets.
- Make destructive or credentialed evidence explicitly opt-in.
- Prefer risk and contract coverage over numeric coverage targets.

When evidence is missing because Mosaico exposes no supported witness, add a
lower-level test and record the observability gap. Do not teach a private table
as public behavior merely to make acceptance testing convenient.
