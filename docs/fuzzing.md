# Backend fuzzing and replay

The portable backend has deterministic stateful/property suites and a Python
supervisor that preserves failures for replay. Run from the repository root
with Python 3.9+ on macOS or Linux:

```sh
python3 scripts/fuzz-backend.py --suite all --seed 1 --cases 100 --steps 1000
```

The Python supervisor depends on POSIX process groups, `wait4`, and per-child
RSS monitoring. It exits with a clear unsupported-platform error on Windows
before building or starting children. The Rust suites and direct replay remain
portable; use the direct Cargo commands below on Windows with a local trace path.

The supervisor builds the release example once and starts a fresh process for
each suite/seed. `--cases` is **per suite**: this command runs 300 children.
Use `--help` for current options and defaults. Useful variations:

```sh
# Reuse a build and allow larger workloads.
python3 scripts/fuzz-backend.py --no-build --suite core --seed 400 --cases 20 \
  --steps 10000 --timeout 180 --rss-limit-mb 4096

# Check whether session teardown releases live allocations.
python3 scripts/fuzz-backend.py --no-build --suite core --seed 1 --cases 10 \
  --sessions 50 --steps 1000 --timeout 300
```

Suites are `core`, `model`, and `projection`; `all` runs each. A fixed binary,
suite, seed and step count reproduce generated actions. Replay records the
actions themselves and remains useful when generation changes. Multiple
sessions advance the seed, so adjacent multi-session cases can overlap.
The timeout covers the whole child, including all sessions.

The supervisor stops on the first finding unless `--keep-going` is set.
Exit 0 means all cases passed; 1 is a child failure, limit violation or invalid
success trace; 2 is a build/configuration problem; 130 is interruption.
A failing child identifies a reproduction to investigate, not necessarily a
confirmed editor defect. Native input and rendering require separate tests.

## Artifacts and replay

Results default to ignored `target/fuzz-campaigns/<timestamp>-<PID>` directories.
`--output` requires a new or empty directory. `--binary` selects a particular
build; the default respects `CARGO_TARGET_DIR`. Keep dirty source changes beside
results: metadata records their status but does not archive their contents.

Each campaign records tool/build identities and a summary. Each case retains its
exact command, flushed action trace, stdout/stderr, sampled memory, exit status
and replay command. The last action is flushed **before** execution, so it may
be incomplete or not yet started when a failure occurs.

```sh
python3 scripts/fuzz-backend.py --no-build \
  --replay target/fuzz-campaigns/CAMPAIGN/00001-core-seed-1/trace.jsonl
```

Use the printed replay command to retain the original binary choice. Replay
preserves the original trace and writes a separate case. Only a torn final JSON
record is omitted; earlier corruption is rejected and omitted bytes are reported.
Neither source documents nor original campaign artifacts are overwritten.

For direct execution without the supervisor's watchdog:

```sh
cargo run --release --example fuzz_backend -- \
  --suite core --seed 1 --steps 1000 --trace /tmp/viem-fuzz.jsonl
cargo run --release --example fuzz_backend -- \
  --replay /tmp/viem-fuzz.jsonl --trace /tmp/viem-fuzz-replay.jsonl
```

## Interpreting memory and limits

RSS is process residency, including allocator/native costs; logical Rust heap
counts requested allocations through the Rust global allocator. High RSS after
teardown can be allocator retention. Repeated growth in comparable session-end
live heap is a stronger signal to investigate. Compare like workloads and report
both metrics. Replay loads its actions, so compare replay memory with replays.
The model suite uses unlimited history for its oracle; its peaks do not measure
the application's default retention policy.

The supervisor samples only its child: `/proc` on Linux, `proc_pidinfo` on macOS
with a `ps` fallback. It also checks the OS-reported child peak. Sampling is a
watchdog, not a hard kernel memory ceiling. Limit violations terminate the owned
process group, escalating to SIGKILL if needed, while retaining artifacts.
A SIGKILL alone is not proof of an out-of-memory kill. If live sampling is
unavailable, the run fails explicitly instead of silently disabling its ceiling.

## Consistency probe

`fuzz_consistency` checks user-visible editing behavior across a corpus of
small Markdown, Code and Text seeds. Every action runs through the real core with
layout. After each case it reopens the saved bytes and requires the same text,
blocks and styles as the edited projection, checks exact undo/redo, and treats
core errors and unexpected statuses as findings. The baseline already contains
known findings, so a run is evaluated by comparison, not by a clean result.

Every core action also runs with shadow verification: each regionally verified
candidate is rebuilt with complete reprojection, and a difference is a
`shadow_mismatch` finding naming the responsible candidate strategy. Every
finding also records, in its `strategy` field, the strategy that verified the
case's most recent edit. `--no-shadow` disables the extra work.

```sh
cargo build --release --example fuzz_consistency
target/release/examples/fuzz_consistency probe --quick --out target/consistency/quick
target/release/examples/fuzz_consistency probe --probes insert,normal,selection,copypaste --out target/consistency/full
target/release/examples/fuzz_consistency compare BASE_DIR NEW_DIR
target/release/examples/fuzz_consistency replay FINDING.json
```

Each probe case has an identity built only from its inputs: format, a hash of
the seed, the probe, its positions and the operation label. A run writes every
failing case to `<probe>-cases.jsonl`, grouped examples to `<probe>-findings.jsonl`
and `<probe>-summary.txt`, and timings to `manifest.json`. Compare cases, not
signature counts: a newly failing case can share a signature with a repaired
one. `compare` groups cases by kind and signature as newly failing (the case
did not fail in the baseline), changed (it fails in both, differently) or
repaired, and exits 1 only when a case newly fails. `--baseline DIR` on a probe run also keeps
complete, replayable findings for newly failing cases in `<probe>-new.jsonl`.

`--quick` runs the insert, normal, selection and copy/paste probes over a
curated subset of the corpus (`QUICK_MARKDOWN`, `QUICK_CODE` and `QUICK_PLAIN`
in the corpus: a few seeds per construct, favoring documents that mix
structures) with four positions per seed. It runs in seconds, about a twentieth
of a full run, and is the feedback pass after changing shared transaction or
projection code. Keep the subset small when adding seeds; the full probe still
covers the whole corpus. Narrow further with
`--filter TEXT` (seed source), `--fmt wysiwyg|code|plain`,
`--op-filter LABEL` and `--probes`. Workers default to all available cores and
start with the largest seeds. Per-case cost is dominated by real editing and
layout, so more threads than cores do not help.

Both sides of a comparison must use the same harness source; case identities
are not comparable across harness changes. `scripts/validate.py consistency
--baseline REV` builds REV in a temporary worktree with the current harness,
removes the worktree and its build when the probe ends, and reuses the results
while the revision, harness and options are unchanged.

## Retained-history benchmark

`history_memory` isolates undo retention from the fuzz oracle. Run each policy
in its own process so cumulative peaks are comparable:

```sh
cargo run --release --example history_memory -- --policy default --steps 1500
cargo run --release --example history_memory -- --policy unlimited --steps 1500
cargo run --release --example history_memory -- --policy 128 --steps 1500
```

It verifies fixed-size UTF-16 edits and reports live/peak heap, nodes, unique
source bytes and the conservative history charge, then prunes and drops state.
The default permits 256 MiB of additional history above live document cost and
10,000 nodes. Shared allocations are charged once; accounting is not an RSS or
instantaneous allocator measurement. Active/open undo states may exceed targets.
For whole-document fixtures, see [performance measurement](performance.md).

## Supervisor tests

```sh
python3 -m unittest discover -s scripts/tests -p 'test_fuzz_backend.py' -v
```

These use temporary children to check exit capture, limits, owned-process
cleanup, replay and artifacts. They require no Rust build. Process-supervision
checks are skipped on Windows; portable trace, replay preparation, and
unsupported-platform checks still run there.

## Coverage-guided parser fuzzing

`fuzz/` is a separate crate of libFuzzer targets for input that Viem does not
control: opened files, pasted clipboard data and syntax-highlighted text. It is
outside the product build graph and runs on the stable toolchain.
`fuzz/build.sh` enables SanitizerCoverage, debug assertions, overflow checks
and AddressSanitizer for both Rust and the native Tree-sitter C code
(`RUSTC_BOOTSTRAP` only unlocks `-Zsanitizer`). AddressSanitizer needs an
upstream clang for C; the script uses Homebrew LLVM when `CC` is unset, because
Apple clang's ASan ABI check does not match Rust's runtime.

| Target | Input | Exercises |
| --- | --- | --- |
| `open_document` | any bytes | every format/encoding/line-ending policy, byte identity, view switch, HTML export |
| `markdown_edit` | bytes + Vim keys | opening with a laid-out view and replaying editing input through commands |
| `clipboard_html` | HTML or private JSON | html5ever tree building, CSS import, payload validation, paste |
| `detect_language` | filename + bytes | modelines, shebangs, filename rules, content signatures |
| `treesitter` | grammar + text | native parsers/scanners, highlight queries, one incremental reparse |
| `syntax_highlight` | language + text | worker provider, injections, Vim fallback and buffer-dependent setup |
| `vim_highlight` | language + text | bundled Vim programs (compiled once) and an incremental edit |

```sh
fuzz/build.sh                         # instrumented binaries in fuzz/target
fuzz/run.sh treesitter 4 3600         # fork mode: workers, seconds
CARGO_TARGET_DIR=fuzz/target-plain fuzz/build.sh --plain
fuzz/target-plain/aarch64-apple-darwin/release/open_document INPUT
```

`run.sh` keeps going after failures and writes them to `fuzz/artifacts/`;
corpora grow in `fuzz/corpus/<target>` (both ignored). The builds are reused by
later campaigns and replays until `fuzz/clean.sh` removes them (also part of
`make clean` and `scripts/clean-win.ps1`); it keeps corpora and findings. Replay an artifact with
the instrumented binary for a sanitizer report, or the `--plain` build to time
it: ASan and coverage are roughly ten times slower, so many campaign timeouts
are not slow without them. Set `VIEM_FUZZ_CONSISTENCY=1` to make
`markdown_edit` also require that saved bytes reproduce the edited projection.
Seed corpora come from repository documents, test fixtures and sources; the
targets document their input layouts. Minimize a failure with
`-minimize_crash=1 -exact_artifact_path=...` and add the result as a test.

Known upstream issue: tree-sitter-swift (through 0.7.4) reads past a directive
string literal when lookahead is 0. The core supplies NUL bytes to parsers as
U+0001, which removes the input-controlled read; a document ending exactly in
`#if`, `#else`, `#elseif` or `#endif` still reads a few bytes of the scanner's
read-only data. `treesitter` skips that case; drop the skip after an upgrade.
