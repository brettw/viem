# Tree-sitter packages and Viem query profiles

This is the initial portable provider profile, revision 1. It is a syntax
decoration service over normalized UTF-8 snapshots, separate from lossless
format parsing, source persistence, commands, and layout.

## Pinned packages

The runtime is tree-sitter 0.25.10. Its parse and query progress callbacks
support cooperative cancellation and explicit incomplete results. Cargo.lock
pins all transitive build inputs.

| Package | Grammar version | Highlight and injection composition |
| --- | --- | --- |
| c | 0.23.4 | nvim-treesitter highlights and injections (NeovimV1) |
| cpp | 0.23.4 | nvim-treesitter C then C++ highlights and injections (NeovimV1) |
| rust | 0.23.2 | Crate highlights and macro injections |
| swift | 0.7.0 | Crate highlights and regex injections |
| objc | 3.0.2 | C then Objective-C highlights; resolved C inheritance |
| c_sharp | 0.23.1 | Copied upstream highlight query |
| javascript | 0.23.1 | Audited JavaScript highlights plus JSX |
| typescript | 0.23.2 | Audited JavaScript then TypeScript highlights |
| tsx | 0.23.2 | Audited JavaScript, JSX, then TypeScript highlights |
| python | 0.23.5 | Crate highlights |

The copied C# query is from
[tree-sitter-c-sharp v0.23.1](https://github.com/tree-sitter/tree-sitter-c-sharp/blob/v0.23.1/queries/highlights.scm).
The JavaScript query starts with
[tree-sitter-javascript v0.23.1](https://github.com/tree-sitter/tree-sitter-javascript/blob/v0.23.1/queries/highlights.scm).
Their upstream MIT licenses are adjacent to the query files.

The C and C++ queries are copied unmodified from
[nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter) commit
40cca05b40438ddd74125132b0cec58c9afdccb2 (`runtime/queries/{c,cpp}`), whose
Apache-2.0 license is adjacent as `c.LICENSE` and `cpp.LICENSE`. They compile
against the pinned 0.23.4 grammars. The C++ files' `; inherits: c` is resolved
by prepending the C query, as nvim-treesitter does. Their injections name
`comment`, `doxygen`, `printf` and `re2c`, which have no bundled provider and so
are not injected (Vim's doxygen syntax does not load in the native profile);
macro bodies inject C/C++ into themselves and raw strings inject the language
named by their delimiter.

JavaScript's two local-variable-sensitive builtin classification patterns are
omitted. Generic identifier/function captures remain. This highlights-only
package does not claim local-variable analysis. Its audited injection query
recognizes tagged-template fragments, regex patterns and JSDoc comments. A
template fragment is an independent language region; it is not silently joined
with unrelated templates. TypeScript uses its own grammar and TSX its separate
grammar. Vim aliases cs, javascriptreact, and typescriptreact resolve to
the corresponding family.

## Validation and matching

Packages select Upstream or NeovimV1 explicitly. Every predicate is rewritten
to a host-handled operator before native compilation, so the binding never
implicitly copies unbounded captured text or silently ignores an unknown
handler. A query is activated only after its grammar nodes, capture arguments,
operators, metadata, and regex dialect validate.

Both profiles support named captures, deterministic integer priority, equality,
membership, text matching, ancestor/parent tests, checked byte-column offsets,
and injection language/combined/include-children declarations. The upstream
profile uses the determinizable Rust regex subset and upstream quantified-capture semantics.
Its DFA compilation has explicit NFA/DFA/determinization memory limits and its
executor charges every byte transition. Unsupported regex features reject the
package rather than falling back to an opaque backtracking operation.
Capture-to-capture equality compares corresponding captured nodes, not their
Cartesian product. Capture/text copying and predicate work share finite budgets.

NeovimV1 pins the relevant behavior to
[Neovim 0.11.4 query.lua](https://github.com/neovim/neovim/blob/v0.11.4/runtime/lua/vim/treesitter/query.lua).
Its match predicates use the bounded Vim matcher, including Neovim's implicit
very-magic prefix policy. Unsupported Vim atoms or magic modes reject the
package. The initial profile rejects quantified predicate/directive capture
arguments, whose any/all and missing-capture behavior differs among Neovim
handlers. Single-capture supported handlers preserve Neovim behavior.
`lua-match?` (with `not-` and `any-`) uses Lua 5.1 patterns over bytes, as
`string.find` does. A pattern without `%b`, `%f` or a back-reference compiles
to the same bounded byte DFA as upstream `match?`; the others run a port of
Lua's backtracking matcher that charges every step to the predicate budget.
Malformed patterns reject the package. `injection.self` injects the package's
own language. Custom Lua, locals scopes, concealment, arbitrary metadata and
unknown directives reject the package; they do not become successful predicates.

Query inheritance and extensions are composition responsibilities of a package
loader. The bundled combinations above resolve their dependencies explicitly;
TreeSitterPackage::compile rejects unresolved inheritance/extension metadata.
A separate platform loader can compose additional query sources, retain a
native library owner, compile the complete package, and register it by language.
Register/unregister changes a generation sampled by the service without an edit.
Retired libraries remain alive through every outstanding package/parser/tree.
Registry capacity is 64 packages.

Capture overlaps resolve by priority, narrower original capture extent,
pattern order, and stable capture order
before stylesheet lookup. The effective style name is the canonical capture
name; origin retains package and capture identity. A name without a definition
takes its nearest defined dotted ancestor's appearance and gets an implicit
definition; it never reveals a losing capture or another provider. Empty completed
queries are exact coverage and clear previous fallback colors.

Known spell/nospell control captures and internal/injection-only captures do
not emit visual styles. A query reload with an unchanged native grammar
preserves its tree and parser continuation; old published snapshots retain
their old package. Changing only the Vim directory reloads fallback programs.

## Parsing, regions and failure

A worker owns each mutable parser. Completed snapshots own separate cloned
native tree handles; raw nodes never cross into persistent editor coordinates.
The parser reads borrowed rope leaves of at most 4 KiB. UTF-8 byte and point
limits are checked before native calls. An edit applies to a private old-tree
copy and carries an exact chain of input identities. Interrupted parsing resumes
only its frozen input, grammar, old tree and included ranges.

An initial parse covers the full host region. A viewport request queries the
completed tree. Declared included ranges preserve parent coordinates for child
parsers. Combined injection discovery queries the complete host under the same
bounded query budget and joins same-pattern, same-language members only after
complete discovery. Incomplete discovery gives missing coverage. As in
Neovim, an injection is kept only when its language has a Tree-sitter package
or a Vim syntax program that loads; the answer is cached per provider and
resolved outside the query's time slice. A captured language that cannot name
a language injects nothing. Child runs are layered over the host's, so host
runs remain in gaps and until the child supplies coverage. Discovery beyond the
injection limit keeps the regions found and reports provisional coverage.
Child Vim input is a bounded isolated region with an explicit range map. Each
worker turn computes up to 64 uncached children or 64 KiB of child input.
Child result caches total at most 4 MiB, and total included child input is
limited to 16 MiB. Host and child parser accounts share a 256 MiB buffer limit.
Compiled bundled packages are shared, so each injected child reuses one query.
A query that misses its time slice is retried in later slices, up to three
times, before it is treated as capped work.

Actual edits are included even when native structural changed ranges are empty.
The present implementation conservatively invalidates an edited suffix instead
of claiming a fine-grained reusable query-dependency index. The service caches a
bounded set of regional results. A capped query additionally remembers its
capture/text/ancestor read envelope, including unsuccessful/capped text reads.
Unrelated edits translate that single memo without restarting identical work.
This envelope permits suppression of failed work only; it is not a proof for
reusing exact query output.

Default limits include 512 MiB input, 2 MiB query source, 32 MiB supplied input
per slice, 16,384 progress callbacks per slice, 65,536 matches, 131,072 captures,
1 MiB copied predicate text, two million predicate steps, 4,096 pending native
matches, 4 MiB raw/final query output, 128 injection ranges, 256 child sessions
and nesting depth 3. Parser
repair is capped at 4,096 total native progress callbacks; cold work is capped
at two million, with at most 4,096 worker slices. Query exhaustion publishes no
partial exact captures. Vim fallback has a finite total instruction allowance.
Failure leaves Vim/default styling and never blocks a required foreground frame.

Before an incremental native edit, the default preflight policy rejects trees
with more than 500,000 nodes or 16,384 root children. These limits are
configurable independently of initial parsing and regional querying. A large
completed tree remains useful for scrolling until an edit requires repair.
Preflight chooses fallback before the measured expensive native tree-edit call;
it does not split arbitrary source into falsely independent parses. Warm repair
failure remembers its originating edit interval, so an unrelated edit or
repaint does not repeat the same capped work.

## Measured native memory and timing

Native allocator hooks are installed once, before any native Tree-sitter
allocation. Platform integrations may provide a language/library owner but
must not create native parser/query/tree handles before calling
initialize_native_accounting. Replacing allocator hooks after allocations
exist is unsupported.

Each native allocation retains its accounting owner in an aligned allocation
header, so reallocations, shared old/new trees, retained snapshots and
cross-thread frees remain attributed. Compilation, parser sessions and transient
queries have separate accounts. Metrics expose retained and peak requested
native bytes, including headers. The default native account limit is 256 MiB;
the aggregate native limit is 1 GiB. These are **soft** limits, checked at
cooperative callbacks and before retaining completed work. Measured excess
cancels/drops owned parse state and selects fallback. The implementation never
returns synthetic allocation failure to a C parser to claim a hard quota.

Allocations made directly by an external scanner outside Tree-sitter's
allocator are not included; allocator overhead and OS resident memory are not
the same as requested bytes. Arbitrary native scanners and native
copy/edit/finalization operations cannot be forcibly preempted in process.
Providers needing enforceable deadlines or comprehensive native memory limits
require process isolation.

The explicit release benchmark is:

    cargo test --release --offline --lib pinned_provider_performance -- --ignored --nocapture

It emits target/treesitter-benchmark.json, with pinned fixture revision,
hardware/compiler metadata, p50/p95/p99 and actual maximum slice/query timings,
native peak bytes, deterministic input/progress/query ceilings, nine-language
10k/100k/1m independent-function fixtures, wide-root statements, and a suspended
100 MiB chunk-input case. It retains every gate failure before returning a
failing test status. Native progress can grow with the number of unchanged
top-level siblings despite constant input bytes supplied; these failures are
reported explicitly and production repairs use the bounded fallback policy.
Provider timing alone does not establish whole-editor input/scroll latency.

The checked-in performance-baseline.json records the diagnostic run on Apple
M1 Ultra (Mac13,2, 128 GiB RAM), macOS 26.6.2, Rust 1.98.1 in release mode.
The diagnostic fixture explicitly disables the production warm preflight
policy and uses a 768 MiB account allowance to expose native behavior. It
records 11 failed exact-provider gates, including all nine million-line
function fixtures, Python's 100k-line fixture, and million-line wide C
statements. Actual maximum native edit slices reached 146.1 ms; the 100 MiB
cold-input case yielded in 4.02 ms after supplying 77,828 borrowed bytes in
chunks of at most 4,096 bytes. These measurements are not a passing exact
large-tree repair claim. The separate production-policy regression requires
finite Vim/default fallback before these expensive edits and no repaint or
unrelated-edit retry.

The production policy report can be reproduced separately with:

    VIEM_SYNTAX_POLICY_REPORT=target/treesitter-policy-benchmark.json cargo test --release --offline --lib native_wide_root_repair_exhaustion_uses_fallback_without_repaint_retry -- --nocapture

The checked-in production-policy-baseline.json records this passing policy
regression. Its single elapsed-time sample is not a percentile benchmark; the
assertions establish finite callbacks, explicit missing/default coverage, and
suppression of retries after repaint and unrelated edits. The diagnostic native
benchmark above remains a distinct failing exact-work gate.
