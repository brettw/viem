# Native Vim syntax profile 4

The native compiler evaluates the bounded setup language below and emits ordered
syntax declarations. Compilation is atomic. An active unsupported command, option, pattern, include,
or nonprogressing include cycle produces a file/line diagnostic and rejects the program.
The directory loader confines transitive includes to the selected syntax root.
Program generations incorporate transitive file names and contents.

The bundled MacVim 9.1.1887 snapshot in `assets/vim/runtime/syntax` supplies
portable runtime fixtures, including `conf.vim`, `dosini.vim`, and nested
`debsources.vim` helpers. The pinned `make.vim` regression fixture is also retained.
An optional differential test uses these sources and a reference Vim executable
(`VIEM_VIM_REGEX_ORACLE`, installed MacVim, or `/usr/bin/vim`) without user
configuration, comparing every non-newline byte's effective highlight group.
This is compatibility evidence for these fixtures, not a claim
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
and zero-width matches are guarded against repeated starts at the same boundary.
Legacy `lc` leading context may reuse preceding text without recoloring it;
the effective match start still obeys the current scanning boundary.
At a zero-width region start, a contained match at that same boundary can obscure
the region's end; `keepend` retains end priority. Without a contained match, an
end at that boundary still closes the region immediately.
Transparent children retain the containing syntax state's highlight when they
extend past a parent's match or highlight-offset boundary.

The compiler accepts line continuations, trailing comments, bar-separated Ex
statements, common command abbreviations, literal heredocs, and the usual
`b:current_syntax` guard. Bounded `if`/`elseif`, `for`/`while`, `break`/`continue`,
`finish`, variable assignment, `unlet`, and saved/restored `cpoptions` support
runtime setup. A `vim9script` header enables hash comments and simple `var`,
`const`, and bare assignments; this does not implement a general Vim9 runtime.
Script-local variables/functions retain separate scopes for included files.
Global helper functions retain their defining script's scope when called later.
Literal `source`, `<sfile>` relative paths, `runtime` wildcards, and syntax
includes stay confined to the selected root. A guarded recursive include may
reenter with different shared setup state; a repeated path/state is rejected.
All recursion also observes the aggregate file and instruction limits.

Setup expressions include scalar/list/dictionary values, indexing, Boolean and
arithmetic operators, comparisons, concatenation, ternaries, and bounded string
and collection helpers. Syntax-generating functions and command wrappers emit
only declarations accepted by the same loader. Function bodies are stored as
bounded data and evaluated only when called during compilation; unused editor
callbacks never run. Helper-generated output is applied in order after the helper
returns and is restricted to syntax/highlight declarations. Generated setup
mutations, includes, control flow, and `hlexists()` after pending output are
rejected because they would require immediate feedback during evaluation.
Active unsupported editor, process, file-writing, or network
commands still reject the whole program. Runtime `try` handlers can handle
supported setup errors; they cannot conceal a compatibility failure.

Highlight group and cluster identities are case-insensitive and retain their
first spelling. `hlexists()` observes native default highlight identities and
groups registered by preceding declarations; color values remain Code policy.
Keyword and region options accept Vim's case-insensitive names,
including Make's `nextGroup`. `syntax clear`, group links/clearing, whitespace in
group lists, and long keyword inventories are supported. Inside a syntax include,
clearing is ignored as in Vim, all new rules are contained, and each include
cluster receives its immediate top-level declarations, including sync group names.
Nested includes retain their own cluster ownership. Large inventories split
into equivalent bounded keyword rules, without increasing per-pattern limits.
Concrete `highlight` attributes establish named-group/link precedence; visual
properties come from the global Code stylesheet.

Buffer-dependent setup receives an immutable normalized source snapshot, a
prefix of the first 32 complete logical lines capped at 64 KiB, and an optional
document filename capped at 16 KiB. The prefix supports Vim9/legacy dialect
selection and range `getline()` queries confined to those 32 lines. With a
snapshot, `line('$')` returns the actual Vim buffer line count and single-line
`getline()` can read any numbered line, including the last one. One terminal
line ending does not manufacture an extra Vim buffer line; preceding explicit
blank lines remain. Each queried line is limited to 64 KiB, with at most 256
queried lines and 256 KiB of retained query data per compilation.

Setup `search()` uses a frozen origin at the first source byte and requires the
non-moving `n` flag. It supports `c`, `w`, `W`, and a stop line without changing
the user's caret. Each snapshot search has a 1 MiB inspected-prefix limit,
one million matcher instructions, a maximum 100 ms deadline, and cancellation;
all calls also share the compilation instruction budget. Exhaustion rejects the
program with a diagnostic rather than returning a guessed no-match result.
These queries neither alter source nor provide general editor access.

Workers track queried line values, line counts, and inspected search prefixes,
including dependencies of failed queries. Setup and cached failures survive
unrelated edits; a changed dependency, prefix, or filename triggers compilation.
After reuse, the context advances to the current snapshot instead of retaining
an old source revision. Viewport changes alone do not rerun setup. Prefix bytes,
filename, and successful query results participate in the program generation
alongside transitive syntax files, so worker eviction preserves interpretation.
Cancelled setup compilations remain retryable and never become cached
syntax-load failures.
Optional external editor capabilities are unavailable. Runtime globals
start absent; declarations within a syntax package may assign bounded values.
The Vim runtime's default embedded-language selections remain in effect.

`sync fromstart`, `minlines`, `maxlines`, and `linebreaks` configure recovery.
Validated sync matches/regions with named `grouphere`/`groupthere` targets,
`linecont`, `ccomment`, and clearing are accepted as recovery hints. Exact scans
still start at document beginning or a validated checkpoint; heuristic recovery
remains explicitly provisional. `display` rules are always evaluated. Folding,
spelling, and concealment declarations are accepted presentation metadata; Code
continues to display every source character and uses its own presentation policy.

## Pattern semantics

The regular subset is compiled to a Thompson NFA and interpreted with explicit
instruction fuel. Regular Unicode patterns that exceed the byte NFA budget can
use the bounded scalar VM, which retains no unused NFA. It does not use an
uninterruptible backtracking matcher.
Supported atoms include literal text, real hard-line anchors, dot and bracket
classes, common Vim ASCII character classes, keyword/identifier classes and word
boundaries, grouping/alternation, greedy and Vim lazy repetitions, explicit
newlines, case overrides, `\zs`/`\ze`, and external region delimiter captures
`\z(` with literal `\z1` … `\z9` end/skip references. Region checkpoints retain
the complete external-capture strings, not just their hashes.

Default magic, nomagic, very magic, very nomagic, and in-pattern mode changes are supported. A resumable compatibility VM handles
postfix lookaround, bounded lookbehind, atomic matches, backreferences, and
conjunctions. Abbreviation atoms, multiline character classes, and document
start/end assertions, numeric character atoms, filename/printable classes,
and supported POSIX classes are also supported. Numeric absolute-line (`\%Nl`)
and byte-column (`\%Nc`) assertions, including `<` and `>` comparisons, query the
immutable source tree in logarithmic time. Line assertions conservatively
invalidate from the beginning after edits. Byte columns count UTF-8 bytes,
independently of tabs, graphemes, and display width. String predicates follow
Vim's string rules: line assertions fail and byte columns are string-relative.
Numeric virtual-column (`\%Nv`) assertions and their `<`/`>` comparisons use
portable Vim defaults: eight-column tab stops, Unicode cell widths with ambiguous
characters one cell wide, and Vim control-character notation. A resumable,
fuel-charged line-prefix scan shares work between increasing assertion positions;
backtracking remains bounded by the same matcher budget. These columns are
independent of native typography and have whole-hard-line edit dependencies.
Numeric character escapes for 0 and 10 follow Vim's buffer/string distinction:
they match NUL in source buffers and LF in string predicates; `\n` matches a
source hard-line boundary. Numeric collection ranges preserve the same rule.
Cursor, mark, Visual-selection, current virtual-column, composing-character (`\Z`/`\%C`),
and substitution-dependent atoms are diagnosed. `setlocal iskeyword` and syntax's
independent `iskeyword` override control keyword classes and boundaries. The final
setup environment applies to all declarations, including dynamically expanded
external-delimiter patterns. Clearing syntax's override restores the buffer option. Pattern matching uses UTF-8; it never
manufactures end-of-line boundaries at rope-leaf or work-slice boundaries.

`VimPattern::compile_neovim_query` follows the Neovim 0.11.4 query predicate
prefix policy: patterns of at least two bytes receive very magic unless they
already begin with a magic-mode switch. Explicit magic-mode switches are honored.
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
of compiled pattern data, 16 KiB per pattern, and 1 MiB/8,192 states per NFA.
The loader bounds diagnostics, statement/conditional depth, logical source lines,
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

It emits `target/vim-benchmark.json`. The benchmark fixtures are unmodified
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

## Runtime compatibility audit

Regenerate diagnostics for the bundled runtime from the repository root:

```sh
cargo run --release --offline --example audit_vim_syntax -- \
  assets/vim/runtime/syntax > target/vim-syntax-audit.json
```

An optional final argument filters relative filenames. The audit reports the
first active compiler failure per file under default setup, without a document
filename or prefix. Independently compiled helper fragments may lack their
owner's context. Compilation success does not establish highlighting equivalence;
use the native Vim differential fixtures for execution parity.

The runtime audit has no document snapshot, so setup requiring `line('$')` or
reads beyond its supplied prefix can still report unavailable context. Provider
regressions load the unchanged bundled `gitcommit.vim` and `diff.vim` with real
snapshots to exercise summary, trailer, comment, scissors, and verbose-diff
highlighting. The included Diff runtime also searches for Hebrew characters and
unified-diff headers throughout the buffer. Large inputs without early matches
can exceed setup's search budget even when a scissors line occurs near the top;
those programs remain unavailable with a bounded diagnostic. Do not replace a
capped search or unavailable context with a guessed no-match, line count, or
empty value. Normal editing and source persistence remain available unchanged.
