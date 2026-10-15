# Development workflow

How to change shared editing code without long rework cycles. The expensive
part of a change is rarely ordinary test execution; it is discovering a boundary
problem late, after builds, measurements and a full consistency run have already
been spent on a candidate that must change again.

## Validation stages

Run the cheapest stage that can find a problem first. Freeze the candidate before
the expensive stages; if new evidence requires a change, reopen implementation
explicitly and repeat the affected stages rather than calling a build final.

| Stage | Command | Use it for |
| --- | --- | --- |
| Type check | `python3 scripts/validate.py check` | Every edit loop |
| Focused tests | `python3 scripts/validate.py focused --filter module::` | The tests nearest the change |
| Quick consistency | `python3 scripts/validate.py consistency --baseline main` | Any change to transactions, projection, typing or paste |
| Suites | `python3 scripts/validate.py suites` | Before measurements or review |
| Native | `python3 scripts/validate.py native` | Frontend, ABI or rendering changes |
| Full consistency | `python3 scripts/validate.py consistency-full --baseline main` | Large or risky refactors, or a request to run all tests |
| Backend fuzzing | `python3 scripts/fuzz-backend.py` ([guide](fuzzing.md)) | Large or risky refactors, or a request to run all tests |
| Memory | `python3 scripts/validate.py memory` | A request to run all tests or about memory, or a change that could plausibly have nontrivial memory impact |

Ordinary changes stop after the suites (and native tests when they apply).
The quick consistency probe is a small curated subset, about a twentieth of
the full probe, that runs in seconds. Full consistency, backend fuzz campaigns
and memory measurements take minutes or have open-ended length; do not run
them for routine changes.

`scripts/validate.py` records a manifest under `target/validation/<timestamp>/`
with the source identity (commit and a digest of uncommitted changes), every
command, its duration, exit status and log, and the artifacts it ran. It finds
binaries from Cargo's JSON output; never run a binary by guessing a path under
`target/`, because a stale artifact from another profile, worktree or
`CARGO_TARGET_DIR` silently invalidates every later result. Run stages directly
when that is simpler; the commands are in the script and in the linked guides.

The core is one large crate, and each Rust test binary recompiles its changed
code separately. After a small edit, `cargo check` is several times faster than
rebuilding a test binary, and the library test binary is the slower of the two
test rebuilds. Use `cargo check` for type errors and one test binary at a time:
`cargo test --test all module::` for integration behavior or
`cargo test --lib path::` for unit tests. Running both rebuilds the core twice.
Do not time anything while another build, test suite or fuzz run is using the
machine.

## Shadow verification

Processes started by Cargo (tests and `cargo run`) compare every regionally
verified candidate with complete reprojection and panic on a difference; see
`.cargo/config.toml` and `src/core/document/candidate.rs`. Ordinary tests
therefore catch locality bugs that previously appeared only in the consistency
probe. Measurement tools turn it off. Set `VIEM_SHADOW_PROJECTION=log` to
report every mismatch without stopping, or `off` to disable it; documents larger
than `VIEM_SHADOW_PROJECTION_LIMIT` bytes (1 MiB by default) are not checked.

A shadow panic names the strategy that produced the wrong candidate and the
first semantic difference (text, block, style span or hard break). It is a bug
in that strategy's proof, not in the test: fix the proof or make the strategy
decline that case.

## Candidate strategies and tracing

Text edits try bounded strategies before complete reprojection. Each attempt
verifies a candidate, declines because it cannot prove locality, or rejects the
edit because its proof covers the whole change and contradicts the request.
Declines are expected and carry a short reason; rejections are final. A strategy
must not use an error to mean "try something broader": return a decline with
`candidate::decline(strategy, "reason")` at the failing gate.

To see why an edit was not regional, wrap it in
`viem_core::document::trace_candidate_strategies` and print the events:

```rust
let (result, events) = trace_candidate_strategies(|| document.insert(at, "x"));
for event in &events {
    eprintln!("{:?} {:?}", event.strategy, event.outcome);
}
```

`verify_regional_candidates` enables shadow verification for one operation and
returns its mismatches instead of panicking, for tests that assert them.

When adding or widening a regional strategy:

- Write the gates first, each with a decline reason, and prove both outer
  boundaries before and after the patches (separator endings, encoding seams,
  list and quote ownership, references and HTML scopes).
- Add locality tests that assert `full_projection_candidates`,
  flattened-text and compatibility-record counters on growing documents, and
  reopen-as-displayed plus exact undo/redo checks for the boundary cases.
- Run the focused tests, then the quick consistency probe against the baseline,
  before measuring anything.

## Explicit materialization

Range stores (blocks, styles, provenance, tables and hard lines) have no
implicit slice view. `compatibility_slice()` and the public whole-projection
accessors `blocks()`, `style_spans()` and `Document::text()` build a flat copy
that the snapshot then retains, including in undo history. Editing paths use
bounded queries (`blocks_for_region`, `style_spans_touching`, `get_range`,
`partition_point`, `items`) or a one-off `to_vec`. Work statistics count each
new copy (`compatibility_record_materializations`,
`formatted_compatibility_materialized_bytes`), so locality tests can assert
that an edit made none.

## Consistency comparisons

The consistency probe is described in [backend fuzzing](fuzzing.md#consistency-probe).
Compare individual cases with the baseline: newly failing cases are the
regressions, including ones that share a signature with repaired cases. Use the
quick probe while iterating on shared transaction code. Run the full probe once
for final evidence only on large or risky refactors.

## Memory and performance claims

Establish allocation ownership before naming a cause. Measure the isolated
change in a fresh process with the allocation probe or a counting allocator,
compare it with the baseline build, and only then describe a proposed fix as
confirmed. A plausible cause that a measurement has not confirmed is a
hypothesis. See [performance measurement](performance.md) for the probes and
checked-in limits; limits are regression inputs and are not raised to make a
change pass. The runners' work ceilings also catch an edit that stays regional
but takes a costlier route, for example when a speculative formulation that
used to fail cheaply now succeeds through a heavier strategy. Run the memory
stage when asked to run all tests or to reduce memory, or when a change could
plausibly have nontrivial memory impact: storage, retained caches or history,
whole-document copies, or a candidate strategy that changes what an edit
materializes. Small fixes to edit translation or escaping do not need it. A
change that deliberately trades one measured cost for another needs the
supporting numbers and a recorded decision.

## Native tests

Native tests need a logged-in GUI session (`launchctl managername` reports
`Aqua`), the Swift toolchain and the system font service. Sandboxed shells
usually lack the GUI session; `scripts/validate.py native` checks this before
building. Its watchdog stops a run that produces no output for ten minutes, and
records the process tree and a sample of each process so a stall in a system
service can be identified instead of waited out.

## Ownership

Give each build, timing run and consistency run one owner. When handing work to
another agent, state which candidate (commit and change digest from the
manifest) it validates and restart an idle agent explicitly. Independent review
and test preparation can run in parallel; measured execution cannot.
