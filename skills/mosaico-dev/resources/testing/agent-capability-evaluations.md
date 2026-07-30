# Agent capability evaluations

Mosaico claims more than protocol correctness. It aims to help agents become
aware of one another, exchange useful information, and coordinate without
unnecessary central control. Those are probabilistic capabilities, not
single-path software contracts.

## What evaluations own

- noticing overlapping work;
- identifying which peer can use a finding;
- making timely useful contact;
- avoiding duplicate or conflicting implementation;
- creating or joining a useful subchannel;
- broadening an investigation toward a shared cause;
- improving final task quality;
- doing so at acceptable latency and cost.

Do not encode these as deterministic Cucumber outcomes.

## Evaluation unit

An evaluation case contains:

- a controlled repository and starting state;
- one or more agent assignments;
- permitted tools, models, harnesses, and budgets;
- relevant hidden or independently scored ground truth;
- expected outcome properties and forbidden outcomes;
- transcript, event, diff, timing, and cost capture;
- cleanup and reproducibility metadata.

The natural-language scenario is input data. It does not require Gherkin or
Cucumber.

## Baselines and conditions

Compare at least:

- agents working without Mosaico;
- Mosaico awareness only;
- awareness plus messaging/coordination;
- more than one relevant model/harness combination where practical.

Use the same tasks, budgets, environment, and scoring policy across conditions.
Without a baseline, a successful run cannot show that Mosaico caused the
improvement.

## Repetition and metrics

Run enough repetitions to expose variance. Record distributions rather than
one green/red result.

Useful metrics include:

- task success and final quality;
- coordination success rate;
- turns/time until relevant contact;
- useful versus irrelevant messages;
- duplicate implementation work;
- conflicting edits;
- whether information reached an actor able to use it;
- latency, tokens, and provider cost;
- robustness across model/harness combinations.

## Scoring

Prefer independent programmatic outcomes when available: tests, repository
state, event relationships, diff overlap, or task artifacts.

For trajectories:

- exact order only when order is itself a policy contract;
- unordered or minimum-required action sets when several orders work;
- rubric or judge scoring for qualitative usefulness;
- multiple judges or human review for load-bearing ambiguous claims.

Never let one implementation agent be the sole author and assessor of its own
evaluation rubric.

## Artifacts and analysis

Retain run configuration, resolved model/harness versions, seeds, transcripts,
Mosaico events, diffs, scores, evaluator comments, latency, and cost. Scrub
credentials and private keys.

Separate:

- framework failure;
- provider/live failure;
- task failure;
- coordination failure;
- evaluator uncertainty.

Do not turn model variance into blind retries. Repetitions are part of the
measurement design and every run remains evidence.

## Promotion

An emergent finding becomes a deterministic test only when it reveals a stable
software invariant. For example, “the peer never received the event” may
produce a relay/delivery regression; “the peer received it but chose not to
respond” remains capability evidence.

Never promote a capability result into deterministic CI because one run—or a
small set of runs—succeeded. Promotion requires a stable software invariant
with a deterministic oracle independent of model choice.
