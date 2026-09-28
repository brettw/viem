# Code syntax implementation and validation

Code is a literal format with an asynchronous decoration pipeline. It shares
the editor's decoded source, persistent text storage, lossless line endings,
commands, and layout. It never runs Markdown/HTML hiding, smart quotes, rich
paste, or source-backed formatting. Syntax results live outside document
history; saving and copying do not serialize them.

## Integration

The portable implementation is in `src/core/document/syntax`, composed by
`src/core/coordinator/syntax.rs`. `SyntaxInputSnapshot` reads borrowed chunks of
at most 4 KiB. Provider results identify their immutable input, configuration,
range, coverage, and named character styles. UI publication validates those
identities and replaces coverage atomically. An exact result with no captures
clears old fallback colors. Unknown winning style names use Code's defaults.

The worker pool has two workers, at most 128 queued buffers, one active and one
coalesced replacement request per buffer, and eight retained idle provider
sessions. Each service retains at most 16 regional results and 4 MiB of runs.
There are finite continuation budgets even if an idle provider is evicted.
Providers, compilation, native-tree destruction, and continuations execute on
workers outside core/mailbox locks. Closing a buffer cancels work without
waiting for a native callback or destructor.

The bundled families are C, C++, Rust, Swift, Objective-C, C#, JavaScript with
JSX, TypeScript with its separate TSX grammar, Python, and JSON (including
JSONC comments). Every bundled package uses pinned nvim-treesitter highlight
and injection queries with their inherited dependencies. Package registration
accepts a validated grammar/query package with a retained native resource owner;
additional platform integrations can provide that owner and use the same
snapshot/coverage interface. Package generation changes invalidate pending
publications. Changing only queries preserves a compatible parse tree.
`Core::set_syntax_provider_factory` also accepts a platform's complete provider
or process proxy; its factory executes on a worker and retains portable
language/configuration state.

The [Tree-sitter profile](../src/core/document/syntax/treesitter/PROFILE.md)
lists pinned grammar versions, upstream/Neovim query support, injected-language
ownership, allocator accounting, and resource limits. The
[Vim profile](../src/core/document/syntax/vim/PROFILE.md) lists the supported
declarations, setup, patterns, synchronization hints, and execution limits.
Unsupported Vimscript or query handlers produce diagnostics and default
styling. This is a declared compatibility subset; it does not execute arbitrary
Vim runtime scripts. Installed MacVim `conf.vim`, `dosini.vim`, `make.vim`, and
legacy/Vim9 `vim.vim` samples have byte-by-byte differential tests against native
Vim. The [installed runtime audit](vim-syntax-audit.md) records the complete
774-file inventory, fixes, reproduction command, and remaining diagnostics.

## Configuration and persistence

The macOS status controls identify Code. Automatic detection can
select it where opening would otherwise select Text; existing Markdown
opening defaults remain. Explicit language selection, safe Vim modelines,
filename associations, shebangs, and bounded content signatures follow the
specification's precedence. The [detection profile](../src/core/document/syntax/detection/PROFILE.md)
exposes validated declarative registration for additional signatures and
extension disambiguators. Ordinary editing does not redetect the language.

The main Settings window has no Code category. The shared Code stylesheet remains
editable through the ordinary modeless Styles inspector opened from a Code view
by F8 or the menu. Syntax load and compiler diagnostics remain available through
the existing document/core diagnostic mechanisms. macOS resolves its directory
from the bundled `Contents/Resources/vim/runtime/syntax` snapshot.
`assets/vim` contains an unchanged copy of the previously configured MacVim
9.1.1887 syntax tree and license, with a SHA-256 inventory. Packaging verifies
the source and destination and replaces the previous resource subtree before
signing, so removed files cannot survive rebuilds. The default remains valid
when the app is moved and needs no installed MacVim.

`config.json` stores an ordered optional `code.filenameAssociations`
array of `{ "pattern": "*.custom", "language": "rust" }` entries. The table
allows at most 256 entries with bounded patterns and language names.
Vim always uses the bundled runtime; there is no directory setting or associated
UI. The retired `code.vimSyntaxDirectory` key is ignored and removed on the next
settings write while unrelated fields are preserved. Syntax includes remain
within the bundled root. Windows resolves the same snapshot from
`AppContext.BaseDirectory/Resources/vim/runtime/syntax`, including after
relocation or launch from another working directory. Build and publish verify
and replace only that resource subtree. See the
[Windows runtime validation](windows-vim-runtime-followup.md).

The selected `themes/<name>.json` file stores the global Code stylesheet in
`styles.code`, using sparse overrides and explicit suppression of deleted or
renamed built-ins. Theme selection lives in `config.json`; both respect
`VIEM_CONFIG_DIR`. The modeless Styles editor targets the current theme and has
its own undo session. Valid file changes update all Code buffers;
invalid external changes preserve the last valid sheet and report a diagnostic.
Color changes reuse shaping, and metrics/paragraph changes invalidate the
corresponding layout while preserving viewport anchors.

Tree-sitter capture names are canonicalized before exact style lookup:
`@comment` uses `Comment`, while `@comment.documentation` uses
`Comment.documentation` with `Comment` as its immediate parent. Only the first
letter is uppercased; the rest of each name is preserved. Dotted default styles
have explicit intermediate parent definitions. Raw capture names remain in
syntax-run provenance for diagnostics.

Code stylesheet version 3 persists these canonical definitions. Version-2
files are migrated in memory without rewriting the file on load. Former capture
overrides take precedence when a capture and a Vim group collapse to one style;
distinct customized names are preserved as user definitions. Migration preserves
valid custom inheritance without introducing cycles. Existing custom names win
new canonical-name collisions, with a unique suffix on the migrated capture's
display name. New saves use version 3. Clearing a built-in declaration is
persisted as inheritance, so it does not restore the original default on reload.

## Reproducible checks

Run the ordinary Rust suite and macOS suite with:

```sh
cargo test --offline --tests --no-fail-fast
scripts/test-mac.sh
```

The native helper builds and signs the current app, forces SwiftPM to relink
the Rust archive, and runs `swift test --disable-sandbox` with an isolated
temporary settings directory.

The full Rust run for the installed-syntax expansion passed 2,559 tests, with
four ignored performance/reference gates and one baseline failure described
below. Focused syntax/provider tests also cover subsequent changes. An earlier
full native run passed all 400 XCTest and 28 Swift Testing tests after a
fresh core rebuild and app relink. It required access to AppKit's save and
pasteboard services, which the restricted execution sandbox denies. The signed
app also passed an isolated launch/process smoke test and verification of its
macOS 26.0 minimum deployment target; no visual UI inspection is claimed.

The final installed-syntax expansion run passed all 155 syntax tests and all
four `EVVimOpeningTests`. The native tests open the installed Makefile and Vim
runtime through the document/backend/paint path, retain exact source bytes and
clean state, and verify that Save As changes filename-dependent setup without
changing document revision. The explicit release Vim performance gate also
passes through one million lines. Its maximum edit repair was 0.911 ms; the
maximum cold slice was 2.993 ms against a cooperative 2 ms target, with bounded
provisional recovery when exact priming did not complete.

The tests cover literal quote ingress and rich-paste stripping, format
source/encoding preservation, exact-name styles,
global persistence/reload, query dependencies and injections, fallback,
supersession, native ownership, finite limits, and Code command paths that
reject whole-document string materialization. Persistent source/text diffs and
line indexes have randomized reference-oracle and large-document tests.

That complete Rust run had one pre-existing rich-caret failure:
`rich_caret_boundaries::every_visible_rich_caret_boundary_accepts_typing_with_either_affinity`.
It also fails in an isolated, unmodified archive of baseline commit
`dfda6f098a427e4269b5d0e1eba997cc8fc44b63`, using its locked dependencies and
`cargo test --offline --test all rich_caret_boundaries::every_visible_rich_caret_boundary_accepts_typing_with_either_affinity`.
Both runs reject inserting `X`, a space, or `é` at the upstream start of a
Markdown list body beginning with backticks with `FormattedPayloadCannotReproject`.
That syntax change did not alter that source-reprojection behavior; its
complete suite was not reported as entirely passing.

The rich-caret failure is now fixed. The ordinary Markdown payload path used
raw upstream source affinity at the first visible boundary, which placed text
before the hidden list marker. It now uses the shared visible-line insertion
resolver; inline affinity still selects context within a line. The original
test passes unchanged, with added exact-patch and undo/redo coverage for that
list boundary and both sides of ordinary, quoted, and list-contained code
bodies.

The follow-up passed all 51 tests across `rich_caret_boundaries`,
`fuzz_markdown_edits`, `markdown_typing_boundaries`, `markdown_structure`, and
`formatted_payloads`, plus `paragraph_menu_insertion` and
`transformation_coverage`; structural newline insertion retains its existing
split boundary behavior. The separate `projection_fuzz_regressions` binary
also passed all 12 tests, confirming its older fence-policy failure was
already resolved.

Explicit release performance commands are documented beside each backend.
The [Vim benchmark](../src/core/document/syntax/vim/performance-baseline.json)
uses pinned `conf.vim`/`dosini.vim` fixtures at 10,000, 100,000, and 1,000,000
lines, plus 4 MiB and 100 MiB hostile lines. Repairs stay within identical
instruction/input ceilings as suffixes grow; distant recovery retains its
explicit provisional label. The reference run's maximum repair was 0.667 ms,
maximum capped fallback 1.096 ms, and maximum cold slice 2.010 ms.
The complete Code pipeline gate is:

```sh
cargo test --release --offline --lib pinned_code_pipeline_performance -- --ignored --nocapture --test-threads=1
```

It holds both real syntax workers in controllably blocked callbacks, then opens,
types, inserts hard lines, undoes, scrolls distant regions, and toggles wrapping
in two views. It asserts bounded detection and chunk reads, no syntax
publication, no whole-document compatibility string during edits, and at most
one pending replacement. The callbacks are released when the fixture exits.
It uses real decoding, projection, and commands with a mock measurement
provider; native AppKit drawing is covered separately.

## Recorded performance and limits

The [pipeline baseline](code-pipeline-performance.json) was recorded in release
mode with Rust 1.98.1 on an Apple M1 Ultra, Mac13,2, with 128 GiB RAM. It records
both syntax-disabled and syntax-enabled runs, 20 input/scroll samples per case,
p50/p95/p99, actual maxima, and fixture construction revision.

| Input | First display, enabled | Input + newline + undo p50 / max | Two-view scroll + wrap p50 / max |
| --- | ---: | ---: | ---: |
| 1,000,000 short UTF-8 lines | 596 ms | 3.34 / 3.67 ms | 0.27 / 0.32 ms |
| 100 MiB UTF-8 | 9.16 s | 5.31 / 9.25 ms | 4.63 / 6.66 ms |
| 100 MiB UTF-16LE | 4.28 s | 4.44 / 8.71 ms | 4.78 / 7.09 ms |
| 100 MiB Latin-1 | 7.59 s | 5.32 / 11.54 ms | 5.76 / 8.09 ms |

Cold loading still eagerly constructs the existing decoded/provenance/block
projection. These measurements establish independence from syntax workers;
they do not establish instant opening of 100 MiB documents. The existing
history accounting estimates roughly 8–16 GB for those live projections
(an estimate, not measured RSS). Because the default 256 MiB history target
would evict undo on such large live states, this benchmark explicitly gives
history a finite budget of the measured live-state estimate plus 64 MiB and
records that override. Compact/progressive baseline projection storage remains
a separate performance limitation.

Ordinary local undo uses persistent source/text differences and avoids whole
document materialization. A history unit containing distant disjoint edits
still reports one conservative source replacement hull; constructing that
history summary can copy the intervening bytes. Its exact anchor/history map
remains independent of the conservative summary.

The [native Tree-sitter diagnostic baseline](../src/core/document/syntax/treesitter/performance-baseline.json)
deliberately disables production warm-edit preflight to expose native behavior.
It records 11 failed exact-provider work gates and a maximum native edit slice
of 146.1 ms. Those diagnostic failures are retained, not presented as passing
exact incremental repair. Production rejects incremental edits of trees over
500,000 nodes or 16,384 root children before that native call, with additional
finite repair fuel and memoized Vim/default fallback. The production-policy
regression passes and requires that unrelated edits and repainting do not
repeat exhausted work. Its [separate policy baseline](../src/core/document/syntax/treesitter/production-policy-baseline.json)
records zero native repair callbacks and zero retries. Large completed trees remain usable for regional
scroll queries until an edit requires repair.

Native allocator hooks record owned and peak requested bytes. The 256 MiB
per-account and 1 GiB aggregate native limits are cooperative soft limits;
measured overruns trigger cancellation/fallback. Arbitrary external scanner
callbacks and native destruction cannot be forcibly preempted in process.
Providers requiring enforceable termination or comprehensive allocation limits
need an isolated process executor.
