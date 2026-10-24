# Development workflow

How to change shared editing code without long rework cycles. The expensive
part of a change is rarely ordinary test execution; it is discovering a boundary
problem late, after builds, measurements and a full consistency run have already
been spent on a candidate that must change again.

## Validation stages

Run the cheapest stage that can find a problem first. Freeze the candidate before
the expensive stages; if new evidence requires a change, reopen implementation
explicitly and repeat the affected stages rather than calling a build final.

Choose validation based on the implementation changed, not every subsystem a
setting can affect. Changes limited to defaults, labels, or settings wiring
normally need focused checks for defaults, persistence, explicit overrides, and
affected native controls. Do not run consistency probes, full suites, fuzzing,
or memory measurements solely because the setting enables an existing feature.
Broaden validation only when implementation logic also changes, focused checks
expose a problem beyond outdated default assumptions, or the user requests
broader coverage. State the concrete reason before broadening. This exception
takes precedence over the general stage guidance below, including the edit-loop
and before-review recommendations.

For Markdown changes, first map the [feature contracts](markdown-feature-contracts.md).
Run small representatives of every affected contract before expanding encoding,
ending and large-document matrices. A candidate is frozen only when the contract
matrix is covered, integration review is complete, and no planned source edits
remain. Say which of these is still pending rather than describing a partial
implementation as final. If validation reopens implementation, name the new
blocking contract and keep adjacent investigations bounded. Record unrelated
defects for follow-up instead of extending the change to repair them.

| Stage | Command | Use it for |
| --- | --- | --- |
| Tooling tests | `python3 scripts/validate.py tooling` | Python validation/build tooling changes |
| Type check | `python3 scripts/validate.py check` | Every edit loop |
| Focused tests | `python3 scripts/validate.py focused --filter module::` | The tests nearest the change |
| Feature contract | `python3 scripts/validate.py contract --contract front-matter` | Representatives across the feature's integration boundaries |
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
`CARGO_TARGET_DIR` silently invalidates every later result. Use the runner for focused work too, so its evidence and time are recorded.
For native selection use `native --native-filter EVCodeBlockLanguageTests`.
Native runs also write `native-phases.json` with separate capability, build,
packaging and execution durations and the artifact reuse/invalidation reason.

Every stage records its relevant input-content digest, toolchain and environment,
test selection, and resolved consistency baseline. Inputs are checked again at
completion; a changed candidate is reported as stale even when its tests pass.
To retain unaffected successful evidence after another change, pass
`--reuse-from path/to/manifest.json`. Only an identical input/environment/options
key with unchanged completion inputs can be reused. Failed, stale or older
manifests without these fields are rerun. Native tests always check capabilities
and execute; their compiled artifacts can be reused separately. Conservative
source groups may rerun more than the minimum; they never authorize skipping a
check based only on a filename guess.

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
`Aqua`), the Swift toolchain, a usable screen, private clipboard access, local
Unix sockets, temporary-file access and the system font service. The runner
probes these capabilities before building. A private clipboard is used so the
probe never replaces user clipboard contents. If the execution environment
cannot provide these facilities, use the authorized native execution environment
before retrying; environment failures are distinct from product regressions.

The native runner separates ABI verification, compilation, packaging and test
execution. It reuses artifacts only when source/configuration/toolchain inputs
and the bytes of every recorded output match. An ordinary filtered run can reuse
a preceding full build. `--no-native-package` (or `test-mac.sh --no-package`)
is for tests known not to require the app bundle; tests that inspect launching,
packaged fonts or resources still require packaging. Raw `--skip-build` is
rejected by the wrapper; it supplies that flag only after verifying artifacts.

Use observable native readiness and completion for successful test paths.
Timeouts are bounded failure guards, not delays that make a test "ready". A
failure that passes on rerun remains an unresolved intermittent failure until
its cause is established; retain the first result and rerun the smallest relevant
group before deciding whether broader validation is needed.

The validator's watchdog stops a run that produces no output for ten minutes,
recording the process tree and samples. It terminates its command group on
interruption so an abandoned build cannot silently keep holding a lock.

## Ownership

Give each build, timing run and consistency run one owner. The validation,
native-test, backend-fuzz and macOS build entry points share an OS-backed lock under
`target/validation/build.lock`. Child builds inherit the same ownership. Waiting
reports the owner's PID and has a finite deadline; manifests record wait time.
Do not delete a held lock file or launch raw Cargo/Swift commands around it.
Independent checkouts have independent locks; measurements still need exclusive
machine resources. When handing work to
another agent, state which candidate (commit and change digest from the
manifest) it validates and restart an idle agent explicitly. Independent review
and test preparation can run in parallel; measured execution cannot.
