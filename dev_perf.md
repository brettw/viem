# Development performance postmortem

Date: 2026-10-13. Scope: the rich-paste performance fix, with context from the
preceding performance work and feature-branch integration.

## Assessment

The largest opportunity is earlier discovery of boundary problems and fewer
repeated implementation and validation rounds. Faster ordinary tests would have
made only a small difference. Candidate preparation also has implicit contracts
and surprising allocation behavior that make performance changes difficult to
reason about.

The source-preservation requirements and broad correctness checks remain
appropriate. They caught real regressions. The improvement should be to find
those regressions earlier and make verification easier to reuse correctly.

This is a blameless analysis of the workflow and its supporting systems. The
agent's investigation and coordination choices contributed to elapsed time;
neither the requirements nor the tests alone explain it.

## Time accounting

The latest rich-paste fix ran from 08:12:50 to 09:53:24 Pacific time on October 13:
**100 minutes 34 seconds**. Conversation metadata records approximately another
1 hour 59 minutes for the earlier performance round and 2 hours 59 minutes for
branch integration, totaling approximately 6 hours 38 minutes across those three
tasks.

The merge's start/end timestamps span longer than its recorded run duration.
The available evidence does not reliably explain that difference; it must not
be attributed to test execution.

The following phases approximate the latest fix's wall-clock timeline. Agents
worked concurrently within phases, so these are not exclusive CPU-time buckets.

| Phase | Approximate duration | Work and findings |
| --- | ---: | --- |
| Initial investigation and implementation | 24 minutes | Baselines, regression probes, regional text/style verification, and list-continuation support |
| First full-suite corrections | 12 minutes | Carriage-return, empty-list, and long-line regressions |
| Allocation investigation and another validation round | 26 minutes | Retained block caches, an unsuccessful table-lookup experiment, the list-owner fix, rebuilding, and measurements |
| First full consistency run | 11 minutes | Six newly rejected whitespace-boundary operations discovered |
| Boundary repair and revalidation | 16 minutes | Command regressions, regional-capture correction, full suites, and refreshed measurements |
| Final consistency run and reporting | 12 minutes | Baseline comparison and final evidence review |

Recorded process times provide a second view:

| Activity | Recorded time | Interpretation |
| --- | ---: | --- |
| Five library and four integration suite runs | 9 minutes | Includes Cargo; actual test execution totaled 4 minutes 52 seconds |
| Two full consistency runs | 22 minutes 9 seconds | Approximately eleven minutes per run |
| Nineteen nontrivial Cargo builds | Approximately 12 minutes 40 seconds | Reported build time; overlaps the suite-command total above |
| Baseline and three valid after-measurement matrices | 4 minutes 11 seconds | Fresh-process benchmark execution |
| Three standard and supplemental allocation-runner pairs | Approximately 1 minute 34 seconds | Summed probe elapsed times |

These rows must not be added into an exclusive wall-time total. Builds, agent
investigation, and other work overlapped. Reasoning, editing, helper preparation,
coordination, and pauses were not comprehensively timed.

The final library and integration suites executed in approximately 64 seconds,
excluding compilation. Halving all recorded ordinary test execution would have
saved only approximately two and a half minutes over the entire patch.

## What the consistency run checks

The consistency probe opens document fixtures, performs editing actions, and
reopens the resulting source bytes as a fresh document. It checks whether the
live document and reopened source agree in text, formatting, and structure,
alongside history and error checks.

For example, deleting a blank line might leave the live editor showing an
ordinary paragraph while the resulting Markdown reopens as a list item. That
is a consistency failure even when the edit returned success.

The requested full run covered approximately four million cases across
insertion, Normal-mode commands, selections, and copy/paste. The baseline already
contains known findings, so the result is a regression comparison. The final
rich-paste run had identical mismatch and operation-error signatures and
occurrence counts to the starting build; it was not a claim that every baseline
case passed.

## Causes of repeated work

### Shared verification changed before its boundary cases were fully exercised

The visible task was removing whole-document copies during paste. The change
also affected shared transaction preparation: hard breaks can change while
visible text stays identical; carriage returns can combine with retained line
feeds; list ownership affects parsing; and blank rows can change how surrounding
separator endings pair.

These interactions emerged in successive test rounds. The first full consistency
run began approximately an hour into the task and found six regressions after
the ordinary suites had passed. Correcting them required another validation and
measurement cycle, followed by another full consistency run.

The existing requirement to verify outer boundaries was correct. The missing
piece was executable coverage for the specific separator and command
interactions, plus a clearer representation of what a local parse had proved.

### Allocation attribution was premature

The agent described a table lookup as the cause of retained-memory growth before
an isolated measurement confirmed it. The experimental change produced no
improvement and was reverted. The actual cause was a list-owner lookup that
populated a whole-document block compatibility cache.

That investigation occupied roughly six minutes of the timeline, with useful
work overlapping it. It should not all be classified as wasted time, but the
causal claim was premature. Allocation ownership should be established before
describing a proposed fix as confirmed.

### Build and measurement orchestration was too manual

One diagnostic helper linked a stale top-level release library instead of the
fresh artifact under Cargo's dependency output directory. The invalid results
were detected, preserved separately, and excluded, but required a rerun.

Temporary scripts repeatedly handled build identities, binary preservation,
measurement, comparison, and reporting. This was useful evidence collection,
but too much of the mechanism had to be assembled during the patch.

### Several validation rounds were called final too early

Locality and allocation questions remained unresolved when broad validation
started. Later corrections then required new builds and measurements. One
handoff also sent messages to an idle agent without restarting it, delaying the
measurement refresh.

The problem was sequencing and explicit ownership of the next action. Once
correctness defects were found, rerunning affected validation was appropriate.

### Failure signatures hid newly failing cases

The initial comparison checked signature sets. Six new rejections reused
existing signatures, so that check alone did not flag them. Occurrence-count
comparison exposed the regressions.

Counts are a useful improvement but remain incomplete: a repaired case and a
newly failing case can cancel each other numerically. Comparisons should retain
stable individual case identities and outcomes.

### The earlier merge had separate native-test problems

The initial native run lacked required services under sandbox restrictions.
Another run stalled in the macOS font service. A pre-existing hover-test fixture
also needed correction. The merge's consistency run then exposed three behavior
defects that required local fixes and revalidation.

Those are separate contributors to the broader elapsed time. Native-service
preflight and bounded stall diagnosis would address them more directly than
making the Rust test suites faster.

## Recommended workflow changes

1. **Exercise shared boundaries early.** After changing transaction verification,
   run focused command tests and targeted consistency probes for whitespace
   separators, `ddP`, selection Enter, CR seams, and list boundaries. Reuse the
   fuzzer's existing seed, format, operation, and probe filters. Retain the new
   command-level regressions; they are inexpensive.

2. **Use one reproducible validation driver.** Discover artifacts from Cargo's
   output, fail immediately on errors, preserve immutable source/build manifests,
   reuse verified baselines, and record phase durations. Keep performance timing
   isolated from concurrent compilation and fuzzing.

3. **Compare individual consistency cases.** Record source, action, selection,
   and relevant configuration identities. Compare outcomes against the baseline;
   keep signature aggregation and example caps for presentation only.

4. **Stage validation deliberately.** Use focused correctness checks, then
   growing-document locality and allocation checks, then freeze the candidate
   for full suites, measurements, and the full consistency run. If new evidence
   requires a correction, reopen implementation explicitly. Do not call a build
   final while known investigation work remains.

5. **Assign one owner to each build and timing phase.** Make handoffs explicit,
   restart idle agents when assigning work, and avoid duplicate builds or timing
   during other CPU-intensive work. Parallelize independent review and test
   preparation while keeping measured execution controlled.

6. **Preflight native requirements and bound stalls.** Detect missing native
   services before a full run. On lack of progress, capture diagnostic evidence
   and identify the stalled phase. Preserve the successful separation of native
   test frameworks where required by shared font-service behavior.

7. **Profile the full fuzzer before optimizing it.** It already uses multiple
   workers and repeatedly opens documents, creates views, and checks layout.
   Investigate a smaller transaction-focused feedback pass while retaining the
   full final check. Do not assume more workers will improve elapsed time.

The immediate objective is one well-prepared final validation cycle. There is
not enough evidence to promise a particular total duration or to claim every
repeat run could have been avoided.

## Architecture assessment

Source authority, disposable projections, persistent storage, and atomic
publication are sound foundations. Encoding seams, hidden syntax, logical-break
identity, and list ownership are necessary complexity for the required behavior.

Candidate preparation has accumulated avoidable complexity. Incremental
refactoring should focus on these contracts:

- **Explicit candidate outcomes.** Distinguish “needs more context,” “proven
  semantic mismatch,” and “verified candidate.” Optional results, ordinary
  errors, and path-specific mismatch variants currently force callers to infer
  fallback policy.
- **A validated regional proof.** Replace a bare pair of old/new ranges with a
  contract that carries edit coverage, coordinate mapping, and boundary
  dependencies. Keep grammar-specific proofs in their owning modules.
- **Shared verification requirements.** Text, logical breaks, styles, and
  retained context should travel with the edit intention and be consumed by
  both regional and complete candidates. Avoid relying on payload shape or call
  order to infer what an earlier path already tried.
- **Explicit materialization costs.** Ordinary-looking iteration, indexing, and
  dereferencing should not unexpectedly retain document-sized compatibility
  caches. Prefer named materialization operations and bounded queries, and
  count compatibility-record materialization alongside text/source work.

These are proposed improvements, not implemented changes or a justification for
a rewrite. Any performance tradeoffs introduced by them still need measurement.

## Proposed AGENTS guidance

AGENTS already states the key source, locality, and verification requirements.
Additional general warnings would add little. Suggested durable additions are:

> An unproven regional capture must remain distinguishable from a proven semantic
> mismatch. Regional verification must account for boundary dependencies before
> and after the edit, including encoding and separator interactions. Performance
> validation must include cold caches, discarded candidates, and retained
> compatibility storage.

Keep build commands, the focused regression matrix, and validation-driver usage
in a development guide rather than expanding AGENTS into an implementation
manual. This postmortem does not change AGENTS or relax existing validation
requirements.

## Evidence and limitations

Task durations came from conversation metadata. Phase boundaries and process
times came from preserved manifests, command logs, and artifact timestamps.
Some artifacts were copied or overwritten during validation, so timestamps
alone were not treated as precise process timings. Reported test/build elapsed
time is not CPU utilization or a complete accounting of agent work.

Local evidence captured for this review:

- `/tmp/viem-rich-paste-20261013/results/core-validation.json`, plus the preserved
  earlier validation manifests and test logs.
- `/tmp/viem-rich-paste-20261013/results/before-whitespace-boundary-fix/consistency-validation.json`.
- `/tmp/viem-rich-paste-20261013/results/consistency-validation.json` and
  `consistency-comparison.md`.
- `/tmp/viem-rich-paste-20261013/results/measurement-report.md`.
- `/tmp/viem-merge-20261012/results/native-validation-summary.json` and
  `integration-report.txt`.

Those temporary artifacts are machine-local and may not remain available.
The time observations above describe this development task, not permanent
product-performance baselines. Raw benchmark measurements remain outside the
repository's performance documentation.
