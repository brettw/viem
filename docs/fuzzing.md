# Backend fuzzing

`examples/fuzz_backend.rs` runs deterministic stateful/property checks against
the portable Rust backend. `scripts/fuzz-backend.py` supervises campaigns,
preserves replay traces, and measures process memory. It uses Python 3.9 or
newer and the standard library on macOS and Linux. Native UI validation remains
a separate check; this runner does not exercise AppKit or Core Text.

## Running a campaign

From the repository root:

```sh
python3 scripts/fuzz-backend.py --suite all --seed 1 --cases 100 --steps 1000
```

The supervisor builds `cargo build --release --example fuzz_backend` once, then
starts a separate child for each suite and seed. `--cases` is the number of
consecutive seeds **per suite**: the command above runs 300 children. The default
suite is `core`; the other suites are `model` and `projection`. A given binary,
suite, seed, and step count generate the same actions. Replay records the actions
themselves, so it also survives changes to the generator.

Default limits are 60 seconds and 2,048 MiB RSS per child, sampled every 0.1
seconds. Override them when exercising larger documents:

```sh
python3 scripts/fuzz-backend.py --no-build --suite core --seed 400 --cases 20 \
  --steps 10000 --timeout 180 --rss-limit-mb 4096 --sample-interval 0.1
```

The supervisor stops at the first finding unless `--keep-going` is set. Exit 0
means every requested case passed; exit 1 means a child failed, exceeded a limit,
or produced an invalid success trace; exit 2 means a build/configuration problem.
An interrupted campaign exits 130 after stopping its active child. A nonzero
child exit is a finding to investigate, not necessarily a confirmed editor bug.

Artifacts default to an ignored directory under
`target/fuzz-campaigns/<UTC timestamp>-<supervisor PID>`. `--output DIRECTORY`
chooses a new or empty directory; existing results are never overwritten.
`--no-build` reuses the release example. `--binary PATH` selects a different
executable, including a separately instrumented build. The default binary
location respects `CARGO_TARGET_DIR`.

## Artifacts and replay

Each case directory contains:

| File | Purpose |
| --- | --- |
| `command.json` | Exact executable and arguments, plus replay-input details |
| `trace.jsonl` | Header, sessions, actions, memory checkpoints, and final result/failure |
| `stdout.log`, `stderr.log` | Complete child output, including panics and diagnostics |
| `memory.csv` | Timestamped resident-memory samples in bytes |
| `result.json` | Exit code or signal, watchdog reason, timing, memory summaries, last action, and replay command |

The campaign's `metadata.json` records compiler/tool versions, Git HEAD and dirty
status, platform/Python version, limits, and the executable's SHA-256 hash.
`summary.json` indexes completed cases and reports the campaign's observed RSS
high-water mark. Build output is saved separately when a build was requested.
Keep the working-tree changes alongside these artifacts when reproducing a
dirty build; the metadata identifies those changes but does not archive them.

Every action is flushed to the JSONL trace **before** it executes. Consequently,
the last action is available even when an invariant fails, the process crashes,
or a watchdog stops it. The last logged action may be incomplete or may not yet
have started, so it is a reproduction candidate, not proof of the fault location.

Replay a finding with the same binary:

```sh
python3 scripts/fuzz-backend.py --no-build \
  --replay target/fuzz-campaigns/CAMPAIGN/00001-core-seed-1/trace.jsonl
```

The printed replay command includes the original binary path. Replay creates a
new case directory and preserves both `replay-original.jsonl` and a separate
`replay-input.jsonl`; output goes to a new `trace.jsonl`. If a kill interrupted
the final JSON record, only that torn final record is omitted from replay input,
and the omitted byte count is recorded. Earlier corruption is rejected. Neither
the original trace nor the source documents used for a campaign are overwritten.

The Rust executable can also be used directly, without the watchdog:

```sh
cargo run --release --example fuzz_backend -- \
  --suite core --seed 1 --steps 1000 --trace /tmp/viem-fuzz.jsonl
cargo run --release --example fuzz_backend -- \
  --replay /tmp/viem-fuzz.jsonl --trace /tmp/viem-fuzz-replay.jsonl
```

## Memory and repeated sessions

Use multiple fresh sessions within one process to distinguish live allocations
retained after document teardown from a single growing document:

```sh
python3 scripts/fuzz-backend.py --no-build --suite core --seed 1 --cases 10 \
  --sessions 50 --steps 1000 --timeout 300 --rss-limit-mb 2048
```

Each case starts at its requested seed; its subsequent sessions use successive
seeds, wrapping as unsigned 64-bit values. Thus adjacent multi-session cases may
overlap seeds. The trace records each actual session seed. `--timeout` covers the
whole child, including all its sessions.

Two different memory measurements are reported:

- **Resident memory (RSS):** the OS memory resident for the child. Linux samples
  come from `/proc/<pid>/status`; macOS samples come from `ps`. `wait4` supplies a
  per-child OS peak even if the child finishes between samples. The observed
  peak is the larger of sampled and OS-reported peaks. First/last ten-sample
  medians and a slope are available for runs with at least 20 samples.
- **Logical Rust heap:** the executable's tracking system allocator records
  live requested allocation bytes and their peak. A `session_end` measurement
  is taken after that session's suite state has dropped. The supervisor reports
  the first and last such measurements, their difference, and average growth
  per completed session. This measures requested allocations, not allocator
  bookkeeping, fragmentation, stack memory, memory mappings, or allocations
  outside the Rust global allocator.

High or increasing RSS alone does not establish a leak: allocators and caches
can retain reusable pages after objects are freed. Repeated session-end live
heap growth is a stronger signal to investigate, especially when comparable
workloads should release their documents, layouts, history, and views. Compare
source-size/action statistics and repeat with more sessions before drawing a
conclusion. A single session has no session-growth estimate. Replay itself loads
its recorded actions, so compare replay memory with equivalent replays, not
directly with generation runs.

Timeout/RSS enforcement is a polling watchdog, not a hard kernel memory limit;
a child can exceed the threshold between samples. The supervisor sends SIGTERM
to its own child's newly created process group, then SIGKILL after a short grace
period, and retains all artifacts. An OS peak over the threshold is also reported
if the child exited before a sample caught it. Unrelated processes are never
selected by name or stopped. The runner does not spawn subprocesses; RSS samples
measure the runner itself. A SIGKILL without an observed limit violation is
reported as SIGKILL, not assumed to be an out-of-memory kill.

If live RSS cannot be read while the child is still running, the supervisor stops
it with `rss_monitor_unavailable`; a requested ceiling is never silently disabled.
For example, a restricted macOS sandbox may forbid `ps`. Run the supervisor in an
environment that permits inspecting its own children. A child that finishes
before its first live sample still has its OS peak checked after exit.

## Supervisor regression tests

```sh
python3 -m unittest discover -s scripts/tests -p 'test_fuzz_backend.py' -v
```

These tests use small temporary child processes to check output/exit capture,
memory ceilings, timeout escalation, descendant cleanup without stopping an
unrelated process, torn-trace replay, and deterministic campaign artifacts. They
do not require a Rust build.

## Retained-history memory benchmark

`history_memory` isolates undo retention from the flat-string fuzz oracle. It
repeatedly changes one character in a fixed-size UTF-16 plain-text document,
checks text/source correctness, and reports live heap, peak heap, history node
count, source-buffer bytes, and the history memory charge. Run each policy in a
separate process so peak counters are comparable:

```sh
cargo run --release --example history_memory -- --policy default --steps 1500
cargo run --release --example history_memory -- --policy unlimited --steps 1500
cargo run --release --example history_memory -- --policy 128 --steps 1500
```

Defaults are seed `20260966` and 3,000 source characters. `--seed` and
`--source-characters` vary the fixture. The benchmark also prunes to one node,
drops the document, and reports both measurements to distinguish retained
history from unreleased session state.

The normal policy remains 10,000 nodes and 256 MiB of retained memory. Its byte
budget includes source/projection trees, provenance indexes, materialized
compatibility views, style payloads, position maps, and history bookkeeping.
The estimate is conservative and counts shared tree allocations once; adding a
snapshot only traverses newly retained tree nodes. The diagnostic
`retained_source_bytes` continues to mean unique source-buffer bytes, while
`retained_memory_bytes` is the budget charge. An oversized active state or an
open undo group's parent/result may exceed the target, and temporary projection
candidates and allocator-retained pages can make process peaks higher.

On the September 2026 validation run, 1,500 edits to the same 6,000-byte source
produced these end-of-run measurements:

| Policy | Retained nodes | Live Rust heap | Retained memory charge |
| --- | ---: | ---: | ---: |
| Default | 1,501 | 55.8 MB | 62.1 MB |
| Unlimited | 1,501 | 55.8 MB | 62.1 MB |
| 128 nodes | 128 | 8.4 MB | 9.3 MB |

MB in this table is decimal; the default 256 MiB target is 268.4 MB. This
workload now fits within the default budget, so it retains the same history as
the unlimited policy. All three runs returned to approximately 2.7 KB live heap
after document teardown.
The model fuzz suite deliberately retains unlimited history for its independent
undo oracle, so its large-session peaks should not be confused with the editor's
default retention policy.
