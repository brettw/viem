# Backend fuzzing and replay

The portable backend has deterministic stateful/property suites and a Python
supervisor that preserves failures for replay. Run from the repository root
with Python 3.9+ on macOS or Linux:

```sh
python3 scripts/fuzz-backend.py --suite all --seed 1 --cases 100 --steps 1000
```

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
cleanup, replay and artifacts. They require no Rust build.
