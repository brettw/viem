# Agent guide for Viem

This guide records requirements and deliberate choices that the code cannot
safely tell you. Detailed behavior, procedures and inventories belong in the
focused guides indexed in [docs/README.md](docs/README.md); link to them rather
than copying them here. Update this file when a requirement changes.

## Product

Viem is a modal, keyboard-first editor for human-language writing: gVim's
composable editing model with word-processor typography (mixed styles,
proportional fonts, soft wrapping, visual-row navigation) and responsive
editing of large documents. macOS and Windows have native frontends; Windows
follows the macOS presentation and behavior using platform controls. Features
are cross-platform unless a difference is listed here.

Each document has one of three source formats:

- **Text** is literal. It is the lossless fallback for every unrecognized file,
  including extensionless files and dotfiles. Every open path, including native
  file-dialog filters, must accept such files.
- **Code** is literal text with optional syntax highlighting.
- **Markdown** is shown formatted in Markdown Preview. Turning Preview off shows
  the same source as Code with the Markdown language.

Changing format, Preview or view mode reprojects the same source and never
converts it: bytes, encoding, BOM, line endings, anchors and undo survive.

Legacy Vim script and Vim9 script are supported for configuration and
automation within the host profile in [docs/vimscript.md](docs/vimscript.md).

## Repository and workflow

- `src/core/document` owns portable document state and policy,
  `src/core/command` interprets Vim input, and `src/core/layout` does layout.
  The crate root composes these services and is the public facade, not a place
  for competing implementations.
- `src/mac` (Swift/AppKit, no SwiftUI) and `src/win` (C#/WinUI 3,
  Win2D/DirectWrite) own platform input, drawing, clipboard, accessibility,
  files and lifecycle. The Rust core never imports a UI framework.
- The C ABI in `include/viem_core.h` is narrow, with explicit ownership and
  batched exchange on hot paths. It and the internal Rust APIs have no backward
  compatibility: change the core, header, Swift and C# callers, generated
  bindings and tests together, delete old adapters, and rebuild both sides
  ([ABI validation](docs/abi-validation.md)). Pointer, snapshot, threading,
  atomicity, persisted-format and user-visible guarantees still apply.
- Use format-family predicates instead of listing format variants at call sites.
- Integration tests go in `tests/all/` and are registered in
  `tests/all/main.rs`; a top-level `tests/*.rs` builds another full-core binary.
  Filter by module: `cargo test --test all ex_sort::`.
- Build and test instructions: [macOS](docs/macos-development.md),
  [Windows](src/win/README.md). After moving a checkout, clean the
  path-dependent Swift/Clang module caches. Builds use the bundled, verified Vim
  runtime, never an installed Vim or a download; preserve its bytes,
  attribution and license.
- Validate through `scripts/validate.py` as described in the
  [development workflow](docs/development-workflow.md). Ordinary changes stop
  after focused tests, the quick consistency probe (for shared editing changes),
  the suites, and native tests when a frontend is affected. Run full
  consistency, backend fuzzing and memory measurements only for large or risky
  refactors or when asked; also run memory measurements for changes with a
  plausible memory impact. Back deliberate memory or performance tradeoffs with
  numbers and record the decision.
- Fuzz builds and campaign results are large and kept until removed. After
  running a fuzz campaign (`scripts/fuzz-backend.py` or `fuzz/run.sh`), end
  your message with a bold line suggesting that the user consider cleaning the
  output: `fuzz/clean.sh` removes the fuzz builds, `target/fuzz-campaigns`
  holds backend results, and `make clean` (`scripts/clean-win.ps1` on Windows)
  removes every build output.
- Tests are deterministic and sit at the document, command, layout and
  coordinator boundaries. Cover source round trips and patch locality,
  incremental-versus-fresh results, encodings and mixed line endings, exact
  undo/redo, every command entry path, failure rollback, reentrant providers,
  and cancellation and stale publication. Keep the architecture dependency
  checks.

## Core architecture

### Ownership and transactions

- `document` owns state, invariants and semantic mutations, and never depends
  on command grammar. `command` reads through immutable public queries; it never
  touches private storage, patches source directly, or mutates state while
  resolving. `layout` consumes document snapshots. Native actions call the same
  model transactions; they never synthesize Vim keys.
- Every source change is a transaction: prepare, tentatively apply and
  reproject, verify, then atomically publish source, projection, history,
  registers, marks, cursor, selection, mode and invalidations. A failed,
  unsupported, cancelled or stale transaction changes none of these. Provider
  callbacks and external I/O never run inside publication. Compound commands
  keep completed steps according to their own partial-progress rules.
- Each buffer has one serial owner of mutable state. Workers receive immutable
  snapshots and return candidates; they never mutate buffers or views or call UI
  code. Don't put a reader/writer lock around the whole model.
- Never hold a lock while parsing, projecting, shaping, wrapping, drawing,
  doing I/O, waiting, or calling callbacks or providers, and never nest cache or
  registry locks. Native render resources have explicit ownership and thread
  rules, including final release from a worker after their view closes.
- A buffer's views share source, history and registers. Each view owns its
  cursor, selection, mode, pending command, desired x, wrapping, line meaning
  and viewport. Closing a view cancels its work and composition and closes its
  insertion undo group without affecting other views.

### Identity and stale work

- Background results install only when every relevant identity still matches:
  document, projection, view, configuration, metrics and resources. A stale
  result never becomes current, and reusing an unchanged subresult requires
  validating all of its dependencies. Identifiers never wrap or get reused.
- Source bytes, decoded text, formatted text and layout are separate coordinate
  domains. A snapshot point is valid only in its document, domain and revision.
  Reject mismatched identities, invalid Unicode boundaries and stale layout;
  never clamp or substitute the current revision. State that outlives an edit
  (cursors, selections, marks, jumps, viewports, history, caches) uses stable
  anchors, not raw offsets.
- Insertion association decides how anchors move across inserted text;
  affinity separately picks the semantic context and visual side of a boundary.
  Selection edges associate inward. Deletion collapses anchors to its boundary,
  preferring the following content. Report ambiguity instead of guessing during
  source edits. Rebasing must not update every anchor per edit or walk
  unbounded map chains.
- Ranges are ordered, half-open and may be empty; keep their direction and
  character/line/block shape. Convert Vim's inclusive endpoints once, before
  operators use them. Block edits are one atomic transaction over a range set.
- Logical edits, selections, search and registers work on extended grapheme
  clusters and are unaffected by fonts or wrapping. Missing glyph geometry may
  widen drawing, never the logical edit. A hard break is one LF in flat logical
  APIs, with no implicit final newline. Objects such as images are atomic, with
  explicit plain-text representations.

### Bounded work

- Work, queues, caches and retained revisions have finite budgets and support
  cancellation. Exact visible work outranks overscan and offscreen work.
  Superseded edits, widths and viewports are coalesced, and foreground editing
  never waits for obsolete work. Cache eviction never affects correctness.
- A local edit costs logarithmic time plus the changed bytes, their
  dependencies, local Unicode and shaping work, and visible reflow. Typing in a
  short line must not scale with document bytes, lines, anchors or visual rows.
- Invalidate only changed dependencies. A global syntax dependency may widen
  the work but must state why. Layout-only changes never invalidate source or
  semantic projections; paint-only changes never reshape or rewrap. Relaying
  out the whole document on each edit is unacceptable.
- Flat whole-store copies (`Document::text()`, compatibility slices,
  whole-projection accessors) are explicit and retained by their snapshot,
  including in history. Editing paths use bounded queries, and locality tests
  assert the materialization counters. Ordinary text must not create per-scalar
  heap mappings, even transiently.
- Opening may scan for detection and indexing, but the first interaction must
  not wait for unrelated whole-document projection, shaping or wrapping.

### Regional strategies

A regional candidate reports verified, declined with a reason, or rejected; it
rejects only when its proof covers the whole change and contradicts the
request. Never use an error to mean "try a broader strategy". A regional proof
accounts for dependencies on both sides of its patches, such as separator
pairing, encoding seams and container ownership. Cargo-run tests shadow-verify
every regional candidate against complete reprojection; any difference is a
defect in the strategy. See
[shadow verification](docs/development-workflow.md#shadow-verification).

### Degraded operation

Failed optional setup and disposable presentation (fonts, themes,
highlighting, decorations, recovery backups) fall back to validated defaults
with bounded diagnostics so that editing stays available. Recovery never
rewrites source, replays input or effects, accepts stale geometry, or hides a
failed verification. See [error recovery](docs/error-recovery.md).

## Source preservation

The source artifact, which may have several parts, is the only persistence
authority. Formatted content, resolved styles and layout are disposable
projections. Every editable pipeline guarantees:

1. **Identity:** opening and saving without an edit reproduces the exact
   artifact, including container metadata.
2. **Locality:** bytes outside the declared patches stay identical, including
   alternative delimiter and entity spellings and whitespace. A small edit never
   regenerates the document.
3. **Correctness:** reprojecting the patched source yields the edit the user
   asked for.

- Retain malformed, unknown and unsupported input losslessly. Serialization
  never depends on the projection, and detection never rewrites input.
- Commands and UI issue semantic intentions against an exact projection
  revision. Supporting syntax repairs may reach beyond the selection but must be
  minimal, explicit patches in the same verified transaction. Never persist an
  edit to a derived span.
- An edit needs an unambiguous reverse mapping through every transform;
  generated or lossy output is read-only. Reverse mapping is relational: don't
  widen an edit to a convenient source hull, drop synthetic or unmapped
  content, or guess from stale identities. Rewrite a shared indivisible
  contributor once, preserving its unselected parts. User-facing errors describe
  the unsupported edit, not internal mapping failures.
- Parsing records its input's origin (whole document, source region, or
  container-stripped fragment), so document-level syntax such as front matter
  can't activate just because a slice begins with its delimiters.
- Replacing a whole encoded artifact and inserting visible text are different
  intentions; an artifact's BOM is not typed U+FEFF.

### Encoding

- Raw bytes are authoritative. Preserve BOMs and declarations, and copy
  untouched bytes instead of re-encoding them. Invalid bytes appear as visible
  diagnostics tied to those bytes; a no-op save never writes U+FFFD.
- New text uses the original encoding when it can. Otherwise use an exact
  format escape (numeric references in Markdown prose, never in code or literal
  views) or reject the edit explicitly. Never substitute characters or change
  the encoding during ordinary input.
- Two exceptions run inside the edit's own transaction: an insertion just after
  a dangling UTF-16 byte may encode U+FFFD to restore alignment, and an edit
  that leaves visible U+FEFF first in a BOM-less document also writes a BOM.
- Explicit encoding conversion is separately undoable and verified, and undo
  restores the exact original bytes. Conversion to Latin-1 may replace
  unrepresentable characters with `?`, reporting the count; ordinary Latin-1
  input stays strict.

### Line endings

- All formats share line-ending handling. `unix` reads LF, `dos` reads CRLF and
  bare LF, and `mac` reads CR. Anything else (other CR/LF, Unicode separators, a
  trailing DOS Ctrl-Z) is content. Opening follows Vim's ordered `fileformats`,
  `unix,dos` by default with `unix` fallback; detection is buffer-local.
- Preserve each existing delimiter and whether there is a final terminator. New
  breaks use the current `fileformat`. Changing `fileformat` converts existing
  breaks in one cancellable, undoable transaction that never reinterprets
  literal CR/LF or adds or removes a final terminator. Reject the conversion if
  reopening would change the logical lines.
- In Text and Code, a final delimiter ends the last line; the empty boundary
  after it is not a line for motions, Ex or numbering. Enter or `o` at an
  unterminated end of file adds the delimiter that keeps the new line, and an
  immediate Backspace restores the original bytes.

## Styles

- Styles are portable configuration with stable identities that don't depend on
  names or array positions. Paragraph (including container) and character roles
  are distinct; character styles never change paragraph geometry. Inheritance
  is single-parent and acyclic. Invalid parents or roles are diagnosed and fall
  back deterministically without discarding source.
- Declarations are sparse: absence inherits, and explicit normal, none, zero or
  transparent values override. Clearing removes the declaration at that layer;
  it never writes a guessed ancestor value. Unknown source declarations survive.
- Resolve paragraph and context appearance first, then the named character
  style chain, then direct formatting. Paragraph percentages use the parent's
  effective size; character percentages use the underlying paragraph size and
  apply once. Base Paragraph allows only absolute sizes. Bold is separate from a
  face's base weight. Unspecified colors resolve through the theme at paint
  time; explicit black or white is not "default". Kerning is always on and has
  no setting.
- **Base Paragraph** is the undeletable root and default paragraph style.
  **Default Paragraph** in the character menu means "no named character style";
  it is not a stored definition. Character styles inherit the current paragraph
  appearance rather than reapplying Base Paragraph (over a heading, for
  example). At most one named character style applies at a point; direct
  properties overlap freely and never modify the named style.
- Deleting a style atomically moves its content and children to Base Paragraph
  or Default Paragraph (not to its parent), or is rejected.
- Document formatting writes only what Markdown can represent. Displaying
  imported or theme properties doesn't make them editable document formatting.

## Markdown

Target GitHub's rendering of repository Markdown: the
[GFM specification](https://github.github.com/gfm/) plus GitHub's documented
features. Missing constructs are compatibility gaps, not alternative rules.
Track them in [MARKDOWN_GAPS.md](MARKDOWN_GAPS.md) and keep
[the feature demo](docs/markdown_demo.md) current. Tables follow
[docs/markdown-tables.md](docs/markdown-tables.md). Before changing Markdown
editing, map the affected
[feature contracts](docs/markdown-feature-contracts.md).

Deliberate presentation differences from GitHub:

- Leading YAML front matter between `---` lines appears as a literal, editable
  YAML Code Block labelled `front matter`, with hidden delimiters and a disabled
  language picker. It never controls editor settings, isn't a selectable
  language, and can't be created from Preview. Unclosed or non-leading `---`
  pairs are ordinary Markdown.
- Reference links and definitions keep their brackets and use the
  `Markdown reference` style (`#A673D1`).
- HTML comments stay visible with the `Comment` style, and surplus blank lines
  remain editable empty paragraphs.
- Images are atomic inline objects; image-only paragraphs use the Image
  paragraph style.
- Embedded HTML is passive semantic formatting: Markdown inside HTML blocks
  stays literal and authored CSS is ignored.

### Passive content and links

- HTML handling (Markdown, clipboard import, entity decoding, export) never
  executes active content, fetches resources, installs authored stylesheets or
  uses a browser to render. Clipboard import may map supported inline CSS to
  resolved styles but never creates an editable HTML document.
- Only local image files are decoded, and only within file-size and dimension
  limits that are checked before reading and before decoding. Remote images
  show their location and are never fetched. A local image reloads only on an
  explicit refresh or a changed resource identity.
- Unknown or hidden content stays anchored to its source. An edit that cannot
  preserve it at an equivalent boundary fails explicitly. Canonicalization
  replaces only the smallest construct the intention needs.
- Only an explicit Open link launches a target. Resolve it from current source,
  relative to the document URL, and reject stale targets, control characters and
  executable schemes. A local file focuses an existing view of that document
  before opening a new window; heading and `#L<line>` fragments navigate within
  it. HTTP(S) opens the default browser. Use native URL and document APIs, never
  a shell.

### Editing in Preview

- Supported deletion and replacement must succeed, including across rows,
  structural boundaries and the ends of code spans; repair delimiters, newly
  active punctuation and folded spaces locally. A verification mismatch on
  supported visible-text deletion is an editor defect. Test every command entry
  path, the reopened source and exact undo/redo, not only prepared edits.
- A caret names a visible location in a paragraph, including an empty one.
  Hidden syntax and list labels have no caret, insertion or deletion stops.
  Line ownership chooses the context at line edges; affinity chooses it inline.
- Insertion takes its character context from caret affinity. Replacement takes
  the first selected text character's context, regardless of direction. Pending
  typing choices win over both. They are view-local until text commits, change
  no source, dirty state or history, never create empty source scopes, and are
  retired by movement.
- New syntax uses `**`/`*` emphasis, ATX `#` headings and `- ` or decimal list
  markers; existing alternative spellings are preserved. Prefer literal
  characters when their meaning is unchanged, escape only where needed, and let
  edits simplify escapes they made unnecessary, within the same undo unit.
- Prose folds single source line endings to spaces. Enter writes two endings in
  the current `fileformat`; Shift-Enter writes an internal hard break
  (backslash, or `<br>` where a physical ending would change structure). Code
  and preformatted text use literal endings.
- Headings, quotes, lists and Code Blocks change through semantic operations.
  List actions apply to whole items with their descendants and renumber
  affected ordered runs in the same transaction, keeping the first run's
  starting number. Removing a treatment preserves paragraph boundaries with
  minimal supporting syntax. Supporting patches share the initiating action's
  verification and undo unit.
- Automatic formatting of typed Markdown (on by default) applies only to
  committed Preview input, uses bounded local context, and shares the
  insertion's undo unit. Protected and opaque content stays literal.
  Ctrl-Q/Ctrl-V literal-next bypasses it, and its literal protection survives
  later edits.
- Toggling Preview is an undoable buffer transaction that keeps bytes and
  recovers each view's caret and viewport. It must not reset scrolling or be
  quadratic; after editing pauses, a background worker prepares the other
  projection.

## Code and syntax highlighting

- Code shows decoded source literally, with no concealment, folding, paragraph
  flow, manual or typing styles, Markdown escaping or prose continuation. Every
  input path keeps supplied quotes. Syntax decorations never reach source,
  clipboard, registers, dirty state or undo.
- Format and language are independent. An explicit format wins on open;
  otherwise recognized code filenames or markers may choose Code over Text.
  Detecting Markdown inside Code never switches to Preview. Language detection
  runs only on load, explicit redetection and filename changes, and an explicit
  language (including None) wins. Markers are data and are never executed. See
  the [detection profile](src/core/document/syntax/detection/PROFILE.md).
- When adding, removing or renaming a bundled language, update the shared
  catalogue, aliases, extensions and special basenames together, following the
  [language maintenance checklist](src/core/document/syntax/detection/PROFILE.md#keeping-language-support-and-filename-rules-in-sync),
  and add detection and opening tests. Vim `.vim` syntax files and Tree-sitter
  grammars are separate backends that register no filename rules; don't derive
  extensions from them. Both frontends' language menus and the Markdown
  code-block pickers use the shared core tiering, in which every language
  appears exactly once.
- Vim script uses the bundled Vim syntax. Other languages prefer Tree-sitter and
  fall back to Vim syntax or default styling when it is missing, incompatible or
  over budget. Fallback fills only unavailable coverage, never gaps between
  captures. Exact, provisional and missing coverage are distinct. Unsupported
  query predicates or Vim atoms report compatibility failures instead of
  highlighting differently. Keep pinned upstream queries unmodified and add host
  support instead. Contracts:
  [Tree-sitter](src/core/document/syntax/treesitter/PROFILE.md),
  [Vim](src/core/document/syntax/vim/PROFILE.md); gaps:
  [highlighter compatibility](docs/highlighter-compatibility.md).
- One Vim script evaluator serves automation, syntax setup and color schemes.
  Hosts supply their permitted effects and query context and never reimplement
  language semantics. Vim syntax patterns, Neovim query predicates and Viem
  Regex v2 are three separate languages.
- Highlighting is bounded to visible work, cancellable and shared across views,
  and never flattens the document or a giant line. Only pre-rendered documents
  (see Layout) are highlighted completely. Before showing newly visible Code,
  frontends may wait up to the shared 100 ms grace period for coverage without
  holding core or provider locks, and never restart the wait for the same
  timed-out request. Publish only current results; invalidation includes text
  predicates, failed matches, context and injections.
- While new highlighting is pending, surviving text keeps its colors and
  inserted text takes the preceding character's style, so typing never flashes
  existing text unstyled.
- Code styles come from one application-wide stylesheet in the selected theme,
  edited with its own undo outside document history (F8 or Edit Styles).
  Providers supply character style names only. Names are case-sensitive;
  Tree-sitter captures drop `@` and capitalize their first letter. Dotted names
  inherit from their dotted parent, and a missing name uses its nearest defined
  ancestor. Accepted syntax creates empty implicit definitions that persist only
  when edited.
- Syntax tests cover cold display with providers suspended, local edits as
  unrelated suffixes grow, distant scrolling, dependencies that reach EOF, huge
  tokens and lines, injections, cancellation, fallback and limits, and compare
  incremental output with fresh analysis.

## Layout

- Core owns layout, wrapping, hit testing and scroll policy. Native shapers
  provide authoritative font resolution, clusters, caret stops and metrics; a
  string-width API is not enough. Fragmenting text preserves shaping context,
  bidi, fallback and ligatures. Geometry is valid only for its snapshot and
  metrics generation.
- The canvas is continuous and unpaginated. Wrapping is per view, never
  changes source, and uses the first-party, untailored Unicode 15.0 UAX #14
  rules ([data policy](data/unicode/15.0.0/README.md)). An unbreakable segment
  overflows, and leading ASCII indentation offers no break. Unwrapped views
  scroll horizontally. `textwidth` affects only explicit reflow.
- A document of at most `FULL_PRELAYOUT_SOURCE_BYTES` (100 KiB) is pre-rendered
  in the background and retained, with exact line heights, table widths, image
  sizes and syntax. Larger documents prepare the same amount of text around the
  viewport and evict the lines farthest from it. Speculative work runs on
  workers below input priority, yields to visible work, never shapes on or
  blocks the input thread, and survives paint-only changes. Retaining geometry
  per view (about 25 MiB per view of 95 KB of Markdown) is an accepted cost.
- Huge hard lines are shaped and wrapped in bounded slices with exact
  continuation context; slice edges never become line breaks.
- Layout-dependent input obtains missing coverage and finishes the same command
  without dropping or replaying the key; retrying layout never repeats edits,
  register effects or undo grouping. Missing coverage is not a document edge.
- Width changes reflow visible rows exactly for the next frame, reusing
  shaping; offscreen heights may stay estimated.
- Scrolling uses continuous layout units and one core-owned extent for wheel,
  keyboard and scrollbars; estimates are not clamps. Preserve the viewport's
  text anchor, and the editing row's screen baseline while it fits, through
  edits, reflow, composition, height refinement and asynchronous font or syntax
  results. Reveal minimally; never recenter or jump to the end. Cache-only
  pre-layout never moves visible text, the caret or the scroll anchor.
- Core decides whether the cursor occupies a grapheme or an insertion boundary;
  frontends never infer it from offset and affinity.
- Visible whitespace markers (`list`/`listchars`) appear only in Text and Code
  and change no text, geometry, search, clipboard or serialization.
- New layout work needs cache-invalidation and large-document tests: a million
  short lines, 100 MiB of mixed-encoding text, multi-megabyte single lines
  wrapped and unwrapped, mixed fonts, bidi and emoji, two views of different
  widths, and rapid resizing followed by editing. Assert work counts, cache and
  queue bounds and stale rejection. Don't raise fixture ceilings to hide
  regressions; timing is not a structural bound and a mock shaper is not
  evidence of native latency. See [performance](docs/performance.md).

## Vim behavior

Follow Vim 9.2 except where listed below. Don't claim a command is supported
until its counts, registers, operator-pending forms, undo grouping and edge
cases are tested (see [the command matrix](tests/all/vim_command_matrix.rs)).
Unsupported commands fail without side effects.

### Line meaning

- Each view uses Visual or Physical Source line meaning (Physical in Code and
  Visual elsewhere, until the user chooses). It governs ordinary line motions
  and operators, Visual Line and line-oriented insertion, including counts and
  replay. Explicit `g` motions stay visual; `gq`/`gw` count hard lines.
- Absolute addresses are always physical source lines: decimal Ex addresses,
  counted `G`, `gg`, `zt`, `zz` and `zb`, `N%`, and launch line numbers.
  Offsets inherit their base's domain: `23+2` is physical line 25, while `.+2`
  advances two displayed rows in Visual mode. `.`, `$`, marks and searches use
  the view's line meaning.
- Native Up/Down always moves by displayed rows. The status position always
  shows the physical line and grapheme column without forcing layout.

### Deliberate differences from Vim

- In Insert Ctrl-N/P completion, any key other than Ctrl-N/P accepts the shown
  candidate and then performs its own action, including Escape, Enter, Tab and
  arrows.
- Normal `U` is unsupported; Visual `U` and `gU` work.
- `modifiable` is buffer-local, defaults on and gates content changes through
  every input path, including undo/redo and encoding/line-ending conversion.
  Changing it is not an edit or an undo entry; `readonly` separately gates saves.
- Vim folding and spelling, `guicursor`, `cursorline`, `colorcolumn`,
  `relativenumber`, `statusline` and `wrapmargin` are deliberately unsupported,
  not a feature backlog. Their recognized compatibility options are documented
  in [the option profile](docs/vimscript.md#deliberately-unsupported-options).
- Text and Code default to `autoindent tabstop=2 shiftwidth=2 softtabstop=2
  expandtab smarttab`, `formatoptions=qj` and `textwidth=80`.
- `=` copies the preceding nonblank line's indentation; it never runs a
  formatter, `cindent`, `indentexpr` or an external process.
- `formatoptions=/` suppresses lowercase `o` comment continuation for every
  trailing line-comment kind, not only `//`.
- Legacy script state (`g:` variables, functions, commands, autocmds),
  mappings and abbreviations are per buffer and shared by its views; `w:`
  variables are per view.
- `:global` selects stable line identities before running. One invocation is
  one undo unit, and per-line failures are reported without stopping it.
- `:s///c` stages its choices against the revision at which the invocation
  starts and applies them together when it ends; an intervening edit cancels
  them. Unlike Vim, the text shown while asking omits the invocation's accepted
  replacements. Inside `:global`, each target's invocation publishes before the
  next target is prepared, so later invocations see earlier replacements, and
  match, replacement-size and work budgets span the whole command. The invoking
  view owns the open undo group between answers, as for Insert typing: a save
  or another writer closes it, and an intervening edit cancels only the current
  invocation and the remaining targets.
- Ctrl-V/Ctrl-Q literal input stores real characters: quoted Return is CR, and
  quoted LF or NUL is U+0000. It bypasses indentation, comment continuation and
  smart quotes.
- Ex filename completion cycles like zsh's menu completion
  ([details](docs/filename-completion.md)).
- `K` runs only a `:`-prefixed `keywordprg`; external programs never start.
  `CTRL-^` edits the pane's alternate file but rejects a count, since there
  are no buffer numbers. `CTRL-G` omits the argument-list position, and `ga`
  reports no digraphs.
- `[(`, `])`, `[{`, `]}` and the method motions match delimiters like `%`,
  without Vim's quote and escape heuristics. `it`/`at` read the formatted
  text, so hidden Markdown HTML has no tag blocks.
- `!` never bypasses the external-change check when writing.
- Native Paste follows gVim's clipboard `gP` behavior (see
  [Input, selection and clipboard](#input-selection-and-clipboard)).

### Search and Viem Regex v2

Search, operator searches, `:substitute` and `:sort` use the stable **Viem Regex
v2** dialect, not Vim regex. Its grammar and rejection cases are pinned by
[search_regex.rs](tests/all/search_regex.rs) and
[regex_commands.rs](tests/all/regex_commands.rs); changes to those tests must
not silently widen or alter the dialect. Keep pattern spelling and dialect
identity in history, repeat and caches.

- Patterns match Unicode scalars, with conventional unescaped groups,
  alternation and quantifiers, Unicode classes and properties, named captures,
  and the `i`, `s` and `x` flags. Vim magic modes, backreferences, lookaround
  and editor-position assertions are rejected by reporting the first
  unsupported atom, not by substring blacklists. `~` is literal.
- `^`/`$` match formatted hard-line boundaries and `\A`/`\z` document
  boundaries; soft wraps and style boundaries are invisible. `\b`/`\B` are
  Unicode word boundaries and `\<`/`\>` their directional forms, independent of
  `iskeyword`. `*`/`#` search for `\b{escaped keyword}\b`.
- Matching is case-sensitive by default; smartcase considers unescaped
  uppercase literals outside classes. Folding is Unicode simple case folding.
- In replacements, `&`/`\0`, `\1`–`\9` and `\g{name}` insert captures, `\n`
  inserts a literal U+000A, and `\r` inserts a semantic hard break.
- Search sees logical formatted text, never hidden markup. If any selected edit
  match starts or ends inside a grapheme, the whole command fails without
  changes; never round endpoints. Matching is linear for a fixed pattern, with
  bounded compilation and work, and is cancellable.
- `incsearch` (on by default) and `hlsearch` (off by default) are shared by a
  buffer's views. Incremental preview never changes the cursor, selection,
  registers or history, and Escape restores the view.

### Registers, repeat and undo

Special register input and mark lifetime follow [registers and marks](docs/vim-registers-and-marks.md).

- Registers keep semantic hard breaks, literal LF and block row separators
  distinct through append and put. Macros record normalized core events with
  their key identity; never parse displayed `<Left>` notation. Putting a macro
  that contains non-text keys fails with `MacroContainsNonTextKeys`.
- Undo/redo restores recorded source artifacts and metadata; it never reruns
  edits or inverse commands. History branches and keeps abandoned futures, and
  redo follows the preferred child. Finalized states are immutable. Derived
  layout and native objects are not history.
- Each of these is one undo unit: a change with its count and all of its
  patches; a change operator with its following Insert input; `o`/`O` with its
  typing; Insert or Replace typing until explicit cursor movement, `Ctrl-G u` or
  Insert `Ctrl-O`; each paste, register insertion and IME commit; each dot,
  macro replay or `:normal` invocation. Failed transactions and no-op edits
  create no history.
- Undo/redo leaves the invoking view in Normal at the recorded anchor, remaps
  other views, and scrolls only as needed to show the change. It doesn't
  restore registers, repeat state, search or prompt history, jumps, view
  options or the filename.
- Dirty state compares the exact persisted source identity, without hashing on
  each query. Saving adds no undo entry, and returning to the saved source is
  clean even if its history node was pruned. Asynchronous saves acknowledge the
  source they captured.
- History retention budgets extra history separately from live state. Prune
  noncurrent leaves before old ancestry, never active states.

### Reflow

`gq` and `gw` rewrap source lines in Text and Code with an internal formatter;
this includes Markdown with Preview off. In Markdown Preview they currently
fail without changes. Widths use portable Unicode columns,
independent of fonts, zoom and window size. Reflow keeps unbreakable words,
list markers, hanging indents and comment leaders, is one undo unit that leaves
registers alone, preserves untouched bytes and line endings, and stays local.
Comment and list handling is in [code formatting](docs/code-formatting.md).

## Input, selection and clipboard

- IME composition is an overlay in Insert, Replace, Select and prompts. Commit
  is one source transaction and undo unit; cancellation leaves no trace.
  Normal, Visual and pending command grammar never expose a native input
  context. Never send partially decoded input to core.
- Smart quotes (off by default) convert straight quotes in every committed
  prose input path, including paste, register puts, IME and accessibility, as
  part of the same undo unit. They never touch Code, code spans or blocks,
  prompts, uncommitted IME text or quotes that syntax requires.
- Native **SELECTION** (`autoselect`, on by default) is separate from Vim Visual
  and Select: native selections are half-open and Vim's are inclusive. See
  [native selection](docs/native-selection.md).
- Native Copy keeps the selection, mode and pending state; it is not a yank.
  `"*c` copies to `*` using yank grammar.
- Native Paste follows gVim's clipboard `gP` helpers
  ([macmap.vim](https://github.com/vim/vim/blob/master/runtime/macmap.vim),
  [paste.vim](https://github.com/vim/vim/blob/master/runtime/autoload/paste.vim)):
  characterwise data goes before the caret and linewise data above the current
  line. Pasting over a selection deletes it into `-` and puts in one undo unit,
  ending in Normal. External text is linewise only if it ends in CR or LF, and
  CRLF or bare CR become semantic breaks.
- In Markdown Preview, external plain text pastes as Markdown. Complete blocks
  keep their structure, a leading or trailing partial paragraph joins the
  surrounding text, and code spans, code blocks and table cells keep the text
  literal. Paste and Match Style, and text that Markdown can't represent, insert
  literally with block syntax protected.
- Preview Copy publishes plain text, native rich text and a versioned private
  fragment; malformed private data never breaks plain paste. Code copies its
  literal source without syntax styles. Windows `+` and `*` share one clipboard.
- Prompt selection, clipboard and register insertion edit only the prompt; they
  never edit the document or submit a command through inserted register text.
- Native toolbar, menu and accelerator actions use the same core intentions as
  Vim commands. Pending Vim grammar, prompts, literal-next and IME composition
  take priority over formatting accelerators.

## HTML export

The core produces a separate, standalone UTF-8 HTML document; frontends only
choose the destination and write the bytes. Export never changes the
document's source, format, path, dirty state or history, and never overwrites an
open document. Markdown Preview exports its formatted structure; Code exports
escaped literal text with full syntax coverage. Output escapes all authored
text, excludes editor furniture, and needs no network resources.

## Files and processes

- One process runs per user. Later launches forward their arguments and working
  directory, and report handoff failures instead of starting another process.
  Names, symlinks and hard links of the same file share one buffer and history.
  Launch helpers preserve the caller's working directory, and development
  `make run*` targets refuse to start while Viem is running. See
  [launchers and the blocking editor](docs/blocking-editor.md).
- At startup only the first file opens; later files load on navigation. A
  nonexistent file opens named, clean and empty without being created.
- Save As adopts the new path only after the write succeeds. No format switch
  or native action renames or deletes the original. Reject symlink and
  case-only aliases that would overwrite the original. Failed writes keep the
  binding and dirty state.
- Watch files with filesystem events and check them off the main thread; never
  poll on a timer. Each changed disk state gets at most one review per buffer,
  shown only while the app is in the foreground, and Keep Buffer acknowledges
  only that change. Every save path checks for external changes and asks
  Cancel or Save Anyway. Reload installs verified
  bytes atomically and keeps each view's position.
- After editing goes idle, recovery writes the complete source and
  interpretation metadata to an exclusive per-document slot, off the UI thread
  and by atomic replacement; it never autosaves over the original. Existing
  swap files offer read-only or cancel, plus Recover for valid Viem snapshots.
  Never interpret, rewrite or delete foreign swap files or a live session's
  slot; remove a stale slot only after a durable replacement snapshot and
  revalidation. Read-only blocks only writes; unforced writes report E45.
- Splits share the document and history but own their cursor, selection,
  viewport, wrapping, line meaning and status. Window commands create no undo
  or register effects. See [split views](docs/window-panes.md).

## Configuration and themes

- Preferences live in one versioned JSON file, `config.json`, in the profile
  directory: an injected path, else `VIEM_CONFIG_DIR`, else `~/.viem`, resolved
  centrally. Validate complete candidates, write atomically, preserve unknown
  keys (including nested ones), and leave invalid or unsupported files untouched
  with a diagnostic. Settings changes create no source edits or document undo.
- Themes hold application-wide appearance and per-format stylesheets, one file
  each in the profile's `themes` directory, with their own settings undo. Named
  themes save automatically; Default is editable in memory only.
  **Default embeds `assets/themes/Midnight.json` at compile time. Tests compare
  the complete preset with the built-in output and validate every packaged
  theme.** Bundled themes are installed for new profiles, and missing ones are
  restored only when the Theme settings section opens.
- A theme variant's style may inherit the same-named style from another
  variant. Cycles are rejected atomically when applied; a cycle found while
  loading falls back to built-in defaults and stays in the file for correction.
- Ignore retired configuration formats; don't add migration code or legacy
  aliases. Save fonts by portable family and subfamily names, base weight and
  variation coordinates, never PostScript names or resource paths.
- Vim color scheme import is described in
  [Vim theme import](docs/vim-theme-import.md).
- `startup.viem` (UTF-8 Vim script, at most 1 MiB) is read once per startup and
  evaluated for each new buffer. It may configure and define but not edit
  documents, open files or change encoding, BOM or `fileformat(s)`. `.vimrc`,
  `.gvimrc`, plugins and runtime scripts are never loaded automatically. See
  [startup commands](docs/macos-development.md#startup-commands).
- The style inspector is one modeless window that edits theme definitions with
  settings undo, never document source. It follows the active view's caret
  without polling, parsing or scanning, and revalidates its target and stable
  style identity before every mutation and callback.

## Native frontends

- Native code adapts input and rendering; it is never a second document,
  selection or undo authority. Accessibility edits use the same checked
  transactions. Test real native event routing, geometry and rendering, not
  just calls to Rust wrappers; rendering or cache changes need real provider
  geometry and cached-versus-fresh pixel comparisons.
- **macOS:** AppKit UI stays on the main thread. Thin carets use the native
  `NSTextInsertionIndicator`; other caret shapes are drawn from core geometry.
  Expose text, ranges and geometry to VoiceOver and text services without
  keeping a native copy of the text. Option-I toggles Italic in Insert and
  Replace when no composition is active, overriding its dead key.
- **Windows:** see [src/win/README.md](src/win/README.md) for behavior and known
  gaps. Ctrl-C/X/V always copy, cut and paste, even during literal-next input,
  so Ctrl-Q replaces Ctrl-V for Visual Block and literal-next. Outside
  literal-next, Ctrl-S, Ctrl-Z, Ctrl-= and Ctrl-- are Save, Undo and zoom, even
  in Insert. Ctrl-0 through Ctrl-5 set Base Paragraph and headings; Ctrl-6 stays
  Vim's Ctrl-^. Other vi control keys reach core, and AltGr stays text input.
  Startup never enumerates all font faces, and shaping uses retained
  DirectWrite font collections.

## Open decisions and work

Don't settle these product questions silently; record decisions here or in an
architecture decision record: additional format adapters, policy for
unrepresentable Unicode edits, Unicode word and sentence tailoring, rich
clipboard obligations beyond those above, unspecified style defaults,
hyphenation and justification, and hardware-specific latency or cache defaults.

Large-file memory use and progressive first display are unfinished; see
[performance](docs/performance.md#open-work). Budget increases or accounting
exclusions don't substitute for compaction.
