# Native Vim syntax profile 1

The native compiler loads declarations; it does not run a Vimscript interpreter.
Compilation is atomic. An active unsupported command, option, pattern, include,
or cyclic dependency produces a file/line diagnostic and rejects the program.
The directory loader confines transitive includes to the selected syntax root.
Program generations incorporate transitive file names and contents.

The installed MacVim 9.1.1887 `conf.vim` and `dosini.vim` files are exercised as
runtime fixtures. When that installation is present, a differential test runs
MacVim without user configuration and compares every non-newline byte's effective
highlight group. This is compatibility evidence for these fixtures, not a claim
that every distributed Vim runtime file compiles. Vim 9.2 remains the normative
behavioral reference.

## Declarations and setup

Supported declarations include ordered keywords, matches, and regions with
multiple starts/ends and one skip, case selection, includes, clusters, group
links, containment and `containedin`, `nextgroup` and whitespace gates,
transparency, per-pattern delimiter matchgroups, `oneline`, `keepend`, `extend`,
and `excludenl`. Offsets `ms`, `me`, `hs`, `he`, `rs`, and `re` count Unicode
scalars, as Vim's pattern offsets do; they are restricted to 1,024 characters of
local work. Retroactive starts/ends that cannot be proven to stay at or after
the current matching boundary, and nonadvancing skip offsets, are rejected;
they cannot relabel already published exact text. Empty endpoints are retained,
and zero-width matches are guarded
against repeated starts at the same boundary. `lc` is unsupported.

The compiler accepts line continuations, the usual `b:current_syntax` guard and
assignment, `finish`, `unlet b:current_syntax`, saved/restored `cpoptions`, and
literal `runtime`/`syntax include` paths. Conditional setup supports literal
`0`/`1`, negation, and the profile's `exists()` environment. Global/unscoped
setup variables are absent. Arbitrary expressions, function calls, functions,
loops, execution, and editor-dependent setup are rejected.

`sync fromstart`, `minlines`, `maxlines`, and `linebreaks` are retained as
configuration/hints. C-comment and explicit sync-pattern recovery are currently
unsupported and diagnosed. `display` rules are always evaluated, so omitting an
optimization cannot alter continuing state. `fold` and `syntax spell` are
accepted metadata; this syntax provider does not fold text or perform spelling
checks. Concealment options are rejected because Code presents literal text.

## Pattern semantics

The regular subset is compiled to a Thompson NFA and interpreted with explicit
instruction fuel. It does not use an uninterruptible backtracking matcher.
Supported atoms include literal text, real hard-line anchors, dot and bracket
classes, common Vim ASCII character classes, keyword/identifier classes and word
boundaries, grouping/alternation, greedy and Vim lazy repetitions, explicit
newlines, case overrides, `\zs`/`\ze`, and external region delimiter captures
`\z(` with literal `\z1` … `\z9` end/skip references. Region checkpoints retain
the complete external-capture strings, not just their hashes.

Default magic and the regular subset of very magic are supported. Nomagic,
very nomagic, backreferences, lookaround, conjunctions, multiline character
classes, arbitrary position assertions, substitution-dependent atoms, and
unsupported escapes are diagnosed. The keyword environment is the native
profile's fixed underscore/digit/alphabetic/Latin-1 environment; editor-specific
`iskeyword` changes are not accepted. Pattern matching uses UTF-8; it never
manufactures end-of-line boundaries at rope-leaf or work-slice boundaries.

`VimPattern::compile_neovim_query` follows the Neovim 0.11.4 query predicate
prefix policy: patterns of at least two bytes receive very magic unless they
already begin with a magic-mode switch. Unsupported explicit modes still fail.
`is_match_text_with_fuel` charges all candidate starts to one caller-owned
predicate budget and reports incomplete work as an error, never a false match.

## Execution and limits

Call the provider on a worker with an immutable `SyntaxInputSnapshot`.
`highlight` returns the checked input identity, requested and actually covered
ranges, named runs, diagnostics, and deterministic work counters. `Exact`
applies only to `covered`; it does not give unprocessed text exact authority.
Calling again with the same input/request resumes the private continuation.
Changing the input without a corresponding `apply_edit` discards old authority.
`highlight_with_control` and `is_match_text_with_control` also check a caller's
cancellation/deadline closure between dispatch steps and inside the NFA every
64 instructions; a stopped matcher retains only its frozen-input continuation.
After an aggregate background-work cap, `discard_pending` abandons that private
continuation while retaining validated checkpoints so a viewport can choose
provisional recovery.

Exact states start at document beginning or a validated complete checkpoint.
Far cold jumps may begin with an empty stack near the viewport and are explicitly
provisional. `fromstart` prevents this heuristic; it does not permit foreground
execution. An oneline region's end/skip preflight is itself resumable. Match
failure and future reads participate in invalidation: ordinary patterns use
whole-line dependencies, and programs with explicit multiline patterns
conservatively invalidate from the beginning.
Cold recovery switches to that provisional path when either the saved-state
distance exceeds 64 KiB or its hard-line distance exceeds the configured
`maxlines` (at least 32). Short lines therefore cannot hide thousands of lines
of cold recovery inside the byte allowance.

Checkpoints use a persistent balanced index, stable buffer identities, and lazy
suffix shifts. Edits do not walk every later checkpoint. Complete equal state
plus unchanged stable context establishes convergence; then a valid downstream
checkpoint can resume work closer to the viewport. Multiple pending edits are
conservatively combined so convergence cannot erase a separate dirty interval.
Checkpoint eviction may require later background repair; it never makes an
interactive frame wait.

Default limits are 4 MiB of setup source, 64 included files, 4,096 rules, 32 MiB
of compiled pattern data, 8 KiB per pattern, and 1 MiB/8,192 states per NFA.
The loader bounds diagnostics and conditional depth, streams physical lines,
and offers cancellation-aware entry points. Cancellation is observed between
bounded compilation steps; NFA construction is bounded by its pattern/NFA caps.

Execution defaults to 200,000 instructions per slice and 8 million instructions
per anchored pattern attempt, including attempts resumed across slices.
Additional configurable caps cover output spans, checkpoint count, region depth,
continuation bytes, external capture bytes, and retained syntax data. A hard cap
returns missing/partial coverage with diagnostics. Capped attempts are memoized
for the request, budget profile, and conservative entry/context dependency;
repaints and independent edits after that dependency do not retry them. Source
snapshots and published result packages are also subject to the shared syntax
service's ownership and memory policy.

## Reproducible performance diagnostic

Run the explicit release gate with:

    cargo test --release --offline --lib pinned_vim_provider_performance -- --ignored --nocapture

It emits `target/vim-benchmark.json`. The checked fixtures are unmodified
`conf.vim` and `dosini.vim` from the installed MacVim 9.1.1887 runtime. Their
original attribution and the accompanying `fixtures/VIM-LICENSE.txt` are
retained. Generated four-line source units, their revision, and workload loops
are pinned in `performance.rs`.

The gate measures character/newline insertion and deletion at the beginning,
middle, and end of 10k/100k/1m-line documents. It uses the same deterministic
instruction, input-byte, line-entry, checkpoint-visit, and retained-byte limits
at every size. Exact background checkpoint priming has a separate finite fuel
budget. The report distinguishes exact repairs from provisional repairs when
priming or checkpoint retention cannot supply an exact nearby state. Every
reported completed viewport is compared with a fresh exact regional execution
including preceding and following section context; cached repaint must perform
zero matcher work. Separate 4 MiB and 100 MiB hostile-line trials measure finite
missing-coverage fallback and require a zero-work memoized repaint.

The report includes compiler/hardware metadata, p50/p95/p99 and actual maximum
slice, repair, fallback, and repaint times. The 2 ms cooperative slice deadline
is a target, not an enforced maximum. Syntax-memory counters exclude shared
input storage and allocator/RSS overhead; compilation and continuation sizes
have additional explicit caps. Twelve repair samples per fixture are a
diagnostic baseline, not a statistical tail-latency guarantee or a whole-editor
input/scroll latency measurement.

The checked-in `performance-baseline.json` records a passing release run on
Apple M1 Ultra (Mac13,2, 128 GB), macOS 26.6.2, Rust 1.98.1. Its 72 completed
repairs include 40 exact and 32 provisional results. The maximum repair took
0.667 ms; the greatest recorded cold slice was 2.010 ms, and hostile-pattern
fallback completed within 1.096 ms. Accounted retained syntax data peaked at
491,718 bytes. Conf's worst repair used 27,751 instructions and 2,887 input
bytes, and dosini's used 19,991 instructions and 1,663 bytes, unchanged across
all three document sizes. Million-line middle/end results remain explicitly
provisional when the finite exact-priming allowance expires.
