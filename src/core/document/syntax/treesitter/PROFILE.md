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
| c | 0.23.4 | C |
| cpp | 0.23.4 | C, then C++ |
| rust | 0.23.2 | Rust |
| swift | 0.7.3 | Swift |
| objc | 3.0.2 | C, then Objective-C |
| c_sharp | 0.23.1 | C# |
| javascript | 0.23.1 | ECMA, JSX, then JavaScript |
| typescript | 0.23.2 | ECMA, then TypeScript |
| tsx | 0.23.2 | ECMA, TypeScript, JSX, then TSX |
| python | 0.23.5 | Python |
| json | 0.24.8 | JSON (including comments) |

All highlight and injection query files are copied unmodified from
[nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter/tree/f603a2f4da48728f80257fb5fbb90145fd1dc173/runtime/queries)
commit `f603a2f4da48728f80257fb5fbb90145fd1dc173`, except Swift's queries,
which use compatible commit `13ddd4d7522ce3e5a1abc0ea34e10ec4e445908a`.
The newer Swift queries require unreleased grammar nodes (including
`nil_literal`); the compatible queries retain upstream's literal `"nil"` rule.
Their Apache-2.0 license is adjacent as `nvim.LICENSE`. Rust and Python's
highlight queries also retain MIT source notices, with the complete texts in
`rust.LICENSE` and `python.LICENSE`. `nvim.NOTICES.md` maps every copied file
to its origin. All four notice/license files are included in macOS and Windows
app output under `Resources/Licenses/nvim-treesitter` (inside `Contents` on
macOS). All packages select the NeovimV1 profile.
`bundled.rs` resolves each file's `; inherits:` dependencies recursively,
prepending each shared query once and removing only those loader headers
from the compiled source. ECMA and JSX are query dependencies, not separately
registered grammars. TypeScript uses its own grammar and TSX its separate
grammar. Vim aliases cs, javascriptreact, and typescriptreact resolve to
the corresponding family.

The Swift grammar was updated to support the nodes used by these queries.
The remaining existing grammars compile the pinned queries without upgrades.
C# directives such as `#if` and `#endif` emit `Keyword.directive`; `DEBUG` in
`#if DEBUG` emits `Constant`. JSON and JSONC filename detection selects the
JSON package, including inside language-tagged injections.

Queries may request child languages such as `comment`, `doxygen`, `printf`,
`regex`, or `jsdoc`. A child is used only when a compatible Tree-sitter package
or loadable bundled Vim syntax is available. Missing children leave the host
highlighting intact. No locals-query scope analysis is claimed.

## Adding or updating a language

Follow the [language maintenance checklist](../detection/PROFILE.md#keeping-language-support-and-filename-rules-in-sync)
in the same change. Tree-sitter support consists of a pinned grammar dependency
in `Cargo.toml`/`Cargo.lock`, query files and inherited dependencies, package
construction/aliases in [`bundled.rs`](bundled.rs), and `BUNDLED_LANGUAGES` plus
availability aliases in [`../treesitter.rs`](../treesitter.rs). Keep those in sync
with this package table and `nvim.NOTICES.md`; preserve upstream query bytes and
include licenses in both native packages. Query helpers are not standalone
languages merely because a `.scm` file exists.

Review the shared catalogue, canonical/Vim fallback aliases and
[`../detection/filenames.rs`](../detection/filenames.rs), and add detection/opening
examples for new filename support. A newly registered Tree-sitter language does
not automatically appear in the catalogue or gain extensions. Adding Tree-sitter
for an already recognized Vim language may need no filename change; verify the
existing mappings and provider/fallback behavior. Vim `.vim` syntax programs are
a separate backend with broader language coverage, not Tree-sitter queries or
grammars. Detection must continue to cover Vim-only languages.

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
Malformed patterns reject the package. Neovim `contains?`, `any-contains?`,
and their `not-` forms perform bounded literal substring searches with Neovim's
all/any semantics. `injection.self` injects the package's own language.

Known `conceal` metadata and capture-specific `bo.commentstring` metadata are
accepted as presentation hints without applying them: Code always shows literal
source, and comment continuation uses its portable language profiles. The
`conceal` capture never emits a visual style or overrides ordinary captures.
Custom Lua, locals scopes, arbitrary metadata and unknown directives still
reject the package; they do not become successful predicates.

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

Native progress can grow with the number of unchanged top-level siblings despite
constant input bytes supplied. Production repairs use bounded fallback when
native edit preflight or repair limits are exceeded. The normal Rust suite
checks incremental-versus-fresh highlighting, local input/query work, native
allocation limits, and this fallback policy.

The historical [performance-baseline.json](performance-baseline.json) records a
diagnostic run on Apple M1 Ultra (Mac13,2, 128 GiB RAM), macOS 26.6.2, Rust 1.98.1
in release mode.
That diagnostic disabled the production warm preflight policy and used a
768 MiB account allowance to expose native behavior. It records 11 failed
exact-provider gates, including all nine million-line
function fixtures, Python's 100k-line fixture, and million-line wide C
statements. Actual maximum native edit slices reached 146.1 ms; the 100 MiB
cold-input case yielded in 4.02 ms after supplying 77,828 borrowed bytes in
chunks of at most 4,096 bytes. These measurements are not a passing exact
large-tree repair claim. Its ignored test was removed because it required exact
repairs beyond the supported production limits. The production-policy regression
requires finite Vim/default fallback before these expensive edits and no repaint
or unrelated-edit retry.

The production policy report can be reproduced separately with:

    VIEM_SYNTAX_POLICY_REPORT=target/treesitter-policy-benchmark.json cargo test --release --offline --lib native_wide_root_repair_exhaustion_uses_fallback_without_repaint_retry -- --nocapture

The checked-in production-policy-baseline.json records this passing policy
regression. Its single elapsed-time sample is not a percentile benchmark; the
assertions establish finite callbacks, explicit missing/default coverage, and
suppression of retries after repaint and unrelated edits. The historical native
measurements do not establish current performance or whole-editor input/scroll
latency.
