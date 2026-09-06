# eVim gaps relative to gVim

This is the prioritized backlog of gVim capabilities that remain candidates
for eVim. It is not a commitment to reproduce every gVim feature. The baseline
is the current `AGENTS.md`, compared with Vim 9.2 and its graphical frontend.
Capabilities omitted from this file are outside the intended scope and must not
be inferred back into the backlog merely for compatibility.

Within that intended scope, the inventory is meant to be exhaustive at the
feature-family level. Vim has hundreds of options, Ex commands, and command
aliases; those should eventually be tracked in generated command and option
matrices rather than copied individually into this document.

## Priority definitions

- **P0 — foundation:** needed for a dependable daily editor or must be designed
  early because it affects core state, command dispatch, persistence, or safety.
- **P1 — important:** common gVim workflows and high-value writing features.
- **P2 — advanced:** valuable but separable from the initial editing experience.

Within a priority, ordering is approximate. The unprioritized programming
section is intentionally excluded from this ranking.

## P0 — foundation gaps

### 1. General option and settings system

`AGENTS.md` specifies `wrap`, `linebreak`, `fileformat`, and `fileformats`, but
not a general Vim-compatible option facility.

- Define typed Boolean, numeric, string, comma-list, and flag-list options with
  validation, defaults, provenance, and change notifications.
- Define global, buffer-local, view/window-local, and global-local values,
  including inheritance when a new buffer or view is created.
- Complete `:set`, `:setlocal`, and `:setglobal`: queries, `all`, changed-only
  display, `no`/`inv`/`!`, reset with `&`, copy global with `<`, assignment,
  `+=`, `-=`, and `^=`.
- Report where an effective value came from, corresponding to `:verbose set`.
- Decide which Vim option names are compatible, translated to eVim concepts,
  ignored with a warning, or rejected. Start with editing, search, wrapping,
  display, clipboard, undo, file-safety, and input options.
- Persist application preferences independently from document source. Define
  precedence among built-in defaults, user preferences, document/buffer state,
  and per-view overrides.
- Add a Settings window for discoverable preferences without making it a
  second settings authority; GUI controls and `:set` must use the same model.
- Define atomic validation and rollback when one command changes several
  options and one value is invalid.
- Define option compatibility/version migration and unknown-option handling.
- Add an option registry that can drive help, completion, Settings controls,
  serialization, and compatibility tests.

### 2. Key mappings, abbreviations, and input dispatch

Basic recorded macros are specified, but user key mapping is explicitly
deferred. Mappings affect the command parser and must not be bolted on after
frontend key handling.

- Support mode-specific mappings for Normal, Visual, Select, Operator-pending,
  Insert, Command-line, language-input, and eventually Terminal modes.
- Support recursive and nonrecursive mappings, unmapping, clearing, listing,
  buffer-local precedence, and origin reporting.
- Define prefix ambiguity and timeout behavior (`timeout`, `timeoutlen`,
  `ttimeout`, `ttimeoutlen`) for both physical and synthetic input.
- Support portable special-key notation, modifiers, function keys, `<Leader>`,
  `<LocalLeader>`, `<Plug>`, `<Nop>`, and literal-key escaping.
- Define mapping modifiers such as `<buffer>`, `<silent>`, `<nowait>`,
  `<expr>`, `<unique>`, `<script>`, and command-form right-hand sides. Features
  requiring an expression runtime may initially return a clear unsupported
  error.
- Preserve the distinction between typed, mapped, macro-replayed, command-line,
  IME, and frontend-native events so remapping and undo semantics are stable.
- Add Insert- and Command-line-mode abbreviations, including mode scope,
  boundary rules, expression forms, listing, and removal.
- Define interruption, recursion limits, diagnostic context, and transactional
  side effects for nested mappings.

### 3. Complete macro and repeat behavior

`AGENTS.md` already requires `q{a-z}`, `q`, `@{a-z}`, `@@`, recursion guards,
and semantic dot repeat. Remaining gaps are:

- Record into numbered, named, unnamed, and append registers wherever Vim
  permits, rather than only lowercase named registers.
- Execute all valid macro registers, including last Ex command and expression
  registers when those registers exist.
- Support counts on macro playback and `@@`, nested playback, and precise
  stop-on-error/interrupt behavior.
- Specify whether mappings are applied while recording and replaying, and make
  this consistent with mapping provenance and `:normal`/`:normal!`.
- Add `@:` for the last Ex command, `:&`/`:~` repeat completeness, search repeat
  offsets, and the full repeat relationship among `.`, `;`, `,`, `n`, `N`,
  `@:`, and substitute repeat.
- Support `:normal!`, Ex execution of a register with `:@`, and repeating the
  last `:@` command.
- Allow users to inspect and edit macro registers without losing normalized
  non-text event identities; define import/export notation for such events.
- Define undo breaks exactly for macro playback containing Insert mode, undo,
  nested macros, failed commands, or frontend actions.
- Add deterministic tests for counts, registers, mappings, recursion,
  interruption, source-projection failures, multi-view state, and dot-repeat
  interaction.

### 4. File safety, external changes, and recovery

- Detect file timestamp, identity, replacement, deletion, permission, and
  external-content changes; implement `:checktime`-like behavior and typed
  reload/overwrite/conflict choices.
- Define autoread policy and notifications without silently discarding edits.
- Support read-only and nonmodifiable states, write permission escalation,
  `:view` behavior, and clear force-write semantics.
- Add crash recovery. Decide whether this uses Vim-compatible swap files or a
  native journal tied to exact source-artifact identity and transformation
  configuration.
- Define backup, write-backup, rename-versus-overwrite, atomic save, fsync,
  symlink, hard-link, metadata, extended-attribute, and file-mode preservation.
- Prevent two processes from unknowingly editing the same artifact; define
  ownership/locking and stale-lock recovery.
- Support incomplete or failed multi-part writes without violating the source
  artifact's identity and patch-locality guarantees.
- Define autosave/recovery snapshots separately from explicit source saves.

### 5. File and buffer lifecycle

- Add the buffer list and alternate-file semantics, including listed, unlisted,
  loaded, hidden, unloaded, deleted, and wiped states.
- Add `:buffer`, `:bnext`, `:bprevious`, `:bfirst`, `:blast`, `:bdelete`,
  `:bunload`, `:bwipeout`, `:buffers`, and useful abbreviations/counts.
- Support `Ctrl-^`/alternate-buffer switching and the alternate-file register.
- Add argument lists and `:args`, `:next`, `:previous`, `:first`, `:last`,
  `:rewind`, `:argadd`, `:argdelete`, and `:argdo` behavior.
- Complete file commands such as `:read`, ranged writes, `:update`, `:file`,
  `:filetype` only if adopted, `:wnext`, `:wprevious`, `ZZ`, and `ZQ`.
- Define current-directory state and `:cd`, `:lcd`, `:tcd`, `:pwd`, and
  previous-directory behavior.
- Support filename modifiers, wildcard/glob expansion, home/environment
  expansion, escaping, command-line filename completion, and `++opt`/`+cmd`
  forms where adopted.
- Specify new-file defaults, reload/re-edit, encoding changes, BOM handling,
  missing final newline, binary mode, and changes to file identity.
- Add recent-files/oldfiles behavior with privacy and persistence policy.

### 6. Command-line and Ex language completeness

The spec currently covers basic command-line editing and a small Ex subset.

- Complete Ex address and range grammar: `.`, `$`, `%`, marks, searches,
  offsets, semicolon versus comma, Visual ranges, and repeated separators.
- Complete command-line editing: word motions/deletion, insert/overwrite,
  quoting, digraphs, register insertion, completion, wildmenu behavior,
  command-line history types, and the command-line window.
- Add command separators, comments, escaping, line continuation, nested command
  parsing, and clear ambiguity rules for abbreviations.
- Add command modifiers such as `silent`, `silent!`, `confirm`, `keepalt`,
  `keepjumps`, `keeppatterns`, `noautocmd`, and `sandbox` as their underlying
  facilities appear.
- Add common general editing commands: `:global`/`:vglobal`, `:sort`, `:retab`,
  `:number`, `:print`, `:list`, `:append`, `:insert`, `:change`, `:center`,
  `:left`, and `:right`.
- Add `:execute`, user-defined commands, `:command`, `:delcommand`, command
  completion, and command provenance if a scripting/expression system is
  adopted.
- Add `:history`, `:clearjumps`, `:changes`, and structured command output that
  can be shown without a terminal grid.
- Define error IDs/categories, warning promotion, prompts, confirmation,
  cancellation, and whether batch Ex commands stop or continue after errors.

### 7. Search, pattern, and substitute parity

- Add `ignorecase`, `smartcase`, `wrapscan`, and related option
  behavior with per-command overrides.
- Add incremental search, search highlighting, clear-highlight, match counts,
  viewport scrolling, and cancellation that restores the original cursor.
- Support search offsets and complete repeat semantics for `/`, `?`, `n`, `N`,
  `*`, `#`, `g*`, and `g#`.
- Complete substitute delimiters, range behavior, flags, confirmation, count-
  only mode, expression replacement, replacement escapes, case conversion,
  previous-pattern/replacement reuse, and error suppression.
- Add `:global`/`:vglobal` interaction with changing lines and nested-command
  restrictions.
- Define search history persistence and privacy.
- Ensure matching across style boundaries, hidden source constructs, generated
  content, objects, and hard-line/paragraph boundaries has explicit behavior.

### 8. Missing core editing command families

The initial command table is substantial but not a complete Vim Normal,
Insert, Visual, and Operator-pending surface.

- Audit every entry in Vim's command index into supported, planned,
  intentionally different, or rejected status, including counts, registers,
  motions, inclusivity, failure behavior, repeat, and undo grouping.
- Add missing general Normal commands such as `:`, `!`, `Ctrl-G`, `Ctrl-L`,
  `Ctrl-^`, `ga`, `g8`, `g<`, `g;`, `g,`, `gi`, `gI`,
  `gq`, `gw`, `g?`, `g@`, `Ctrl-A`, and `Ctrl-X` where they make sense.
- Complete section, sentence, paragraph, bracket, method-like, and unmatched-
  delimiter motions (`[[`, `]]`, `[]`, `][`, `[(`, `])`, and related forms),
  distinguishing prose semantics from source-code semantics.
- Add missing text objects, notably tag blocks (`it`/`at`), with a clear rule
  for formatted HTML versus literal/plain text.
- Add formatting operators and options (`gq`, `gw`, `textwidth`,
  `formatoptions`, paragraph joining and reflow) adapted to proportional layout
  and source-preserving formats.
- Complete indentation, shifting, case conversion, ROT13 if retained, numeric
  increment/decrement, character inspection, and replace/Virtual Replace
  semantics.
- Complete put variants, cursor placement, line/block payload edge cases,
  register rotation, and rich-format paste behavior.
- Complete Insert/Replace special keys: literal insertion, digraph entry,
  register/expression insertion variants, indentation controls, completion,
  and insert-repeat commands.
- Add Select mode or explicitly classify it as a non-goal distinct from Visual
  mode and native selection behavior.
- Decide which terminal-column concepts—`virtualedit`, tabs as screen cells,
  display columns, block padding—translate to pixel geometry and which are
  deliberately incompatible.

## P1 — important gaps

### 9. Registers, marks, jumps, and change navigation

- Complete special registers: last Ex command (`:`), last search (`/`),
  expression (`=`), alternate file (`#`), drag/drop, and any GUI-selection
  aliases not already covered.
- Add `:display` compatibility and richer safe inspection/editing of text,
  linewise, blockwise, formatted, and macro payloads.
- Add uppercase/global file marks, numbered marks, and special marks for last
  jump, last edit, last insertion, change/yank bounds, last Visual selection,
  and last exited position.
- Complete exact-line versus exact-position mark jumps and mark deletion/listing.
- Add the change list and `g;`/`g,`, complete jump-list mutation rules, and
  jump-list traversal across buffers.
- Expose the existing branching undo model through `:undolist`, `g-`, `g+`,
  `:earlier`, and `:later`, including time/count units and branch selection.
### 10. General GUI interaction and appearance

- Complete mouse semantics: multi-click selection, word/paragraph selection,
  shift/extend, drag-and-drop text and files, selection autoscroll, contextual
  menus, and mode-sensitive right-click behavior.
- Define primary-selection versus clipboard semantics on platforms that have
  both; on macOS, clarify how Vim's `*` and `+` aliases map to one pasteboard.
- Add configurable line numbers, relative numbers, cursor line/column,
  whitespace and end-of-line markers, nonprinting characters, margins, and
  scroll offsets using proportional-layout geometry.
- Complete accessibility exposure for text ranges, styles, selections,
  line/paragraph navigation, hidden content, and very large documents.

### 11. Formatting compatibility beyond current style controls

- Specify Vim-style plain-text formatting options such as `textwidth`,
  `wrapmargin`, `formatoptions`, `comments`, `joinspaces`, and `paste` where
  they remain meaningful for human-language documents.
- Define `gq`/`gw`, hard wrapping versus soft wrapping, paragraph reflow,
  alignment commands, and undo/provenance behavior for source formats.
- Define tabs, indentation, list continuation, and blockwise edits in terms of
  layout x positions without pretending proportional text has character cells.
- Add formatting search/replace and style-copy mechanisms not present in gVim
  but expected in a word processor.
- Define import/export or fallback behavior for unsupported character,
  paragraph, object, page, and list properties in every adopted adapter.

## P2 — advanced gaps

### 12. Automation and extensibility runtime

- Decide whether to implement Vimscript, Vim9 script, a different embedded
  language, a typed automation API, or no general in-process scripting.
- If an automation runtime is adopted, define values, scopes, expressions,
  functions, exceptions, modules/imports, callbacks, timers, and safe access to
  source versus formatted coordinates.
- Add autocommands/events with ordering, nesting, cancellation, recursion,
  buffer/view scope, and transactional rules.
- Add user commands, expression mappings/registers, custom operators (`g@`),
  completion functions, and status/highlight hooks through stable APIs.
- Add plugin/package discovery, versioning, dependencies, enable/disable,
  updates, crash isolation, capability permissions, and trust UI.
- Define a runtime path or native replacement for loading plugin resources.
- Ensure automation cannot retain stale numeric offsets or bypass verified
  reverse projection and atomic source transactions.

### 13. External commands, filters, jobs, channels, and terminal

- Add `:!`, shell escapes, ranged filters (`!{motion}` and `:{range}!`),
  `:read !`, and `:write !` with encoding, cancellation, stderr, exit status,
  and undo semantics.
- Define shell selection, quoting, working directory, environment, sandboxing,
  and explicit user consent for untrusted documents/configuration.
- Add asynchronous jobs, pipes, callbacks/channels, cancellation, and output
  buffering without blocking the UI.
- Add terminal buffers and Terminal-mode input only if they fit the product;
  preserve the buffer/view distinction and do not contaminate formatted-source
  semantics.
- Add process/session cleanup, crash handling, resource limits, and clear
  behavior when closing a document or app with running jobs.

### 17. Collaboration between application instances

- Add client/server or native interprocess open-file routing only if needed.
- Define remote command execution, wait-for-file completion, focus/raise,
  ownership of modified buffers, and authentication.
- Avoid accepting arbitrary remote Ex or script execution by default.

## Unprioritized syntax-highlighting and programming-language gaps (FOR REFERENCE, DO NOT IMPLEMENT)

These are intentionally not assigned product priorities. They should not drive
the architecture of the human-language editor, but the architecture should not
gratuitously prevent future implementations.

### Syntax and filetype system

- Syntax highlighting rules, regions, matches, clusters, containment,
  synchronization, spell regions, concealment, and `:syntax` commands.
- Highlight-link conventions and compatibility with Vim syntax highlight
  groups.
- Filetype detection by name, content, shebang, and user override.
- Filetype plugins, after-directories, buffer-local defaults, and `b:did_*`
  loading conventions or a native replacement.
- Syntax-derived folds and language-aware selection/text objects.
- Incremental parsing/tree-sitter integration as an alternative to Vim's regex
  syntax engine, with explicit behavior for malformed and huge files.

### Indentation and formatting for code

- `autoindent`, `smartindent`, `cindent`, `cinoptions`, `indentexpr`, and
  filetype indentation scripts.
- `=` operator integration with language indentation and external formatters.
- Comment leaders, code-aware joining/reflow, and language-specific text
  objects.
- Formatting-on-save or formatting-on-type, with source-preserving conflict and
  undo policy.

### Code navigation and symbols

- Tags and tag stacks: `Ctrl-]`, `g]`, `:tag`, `:tselect`, `:tjump`, split/tag
  variants, and tag-file discovery.
- Cscope-style queries, include-file searches, declaration/local-declaration
  motions, definition search, and filename-under-cursor commands such as `gf`.
- Symbol outlines, breadcrumbs, references, rename, hover, diagnostics, and
  other LSP-like features not built into classic gVim.
- Match-pair and structure navigation extensions such as `matchit`.

### Build, grep, and error navigation

- `:grep`, `:lgrep`, `:vimgrep`, recursive search, grep programs, and result
  population.
- Per-project working directories, environment, task configuration, and build
  cancellation.

## Explicit product differences and likely non-goals

These are gaps in literal gVim compatibility but are already implied by the
product direction and should not be mistaken for omissions:

- With wrapping enabled, `j`/`k` use visual rows by default; gVim distinguishes
  logical-line and screen-line commands.
- Proportional shaping, grapheme boundaries, bidi caret affinity, variable row
  heights, and pixel x positions replace terminal character-cell assumptions.
- The formatted document is a projection of an authoritative, losslessly
  preserved source artifact; gVim primarily edits buffer text directly.
- Rich character/paragraph styles and format-aware semantic edits intentionally
  exceed gVim's document model.
- A pixel-identical gVim UI and every historical Vi quirk are not current
  product goals.
- The macOS application uses native menus and caret presentation. It has no
  specified toolbar and should not acquire one solely for gVim parity.
- Windows and terminal frontends remain future possibilities, not current
  deliverables.

## Follow-up audit artifacts

To turn this inventory into testable commitments, create these separate
machine-readable matrices when priorities are selected:

- every Normal/Visual/Insert/Replace/Operator-pending command and key;
- every Ex command, abbreviation, address/range form, and modifier;
- every register, mark, history, and repeat target;
- every Vim option with scope, type, eVim mapping, and support status;
- every GUI command/interaction with native replacement or non-goal status.

Each entry should be classified as `supported`, `partial`, `planned`,
`intentional-difference`, or `rejected`, and should link to behavioral tests
when support is claimed.

## Primary comparison sources

- [Vim 9.2 help overview](https://github.com/vim/vim/blob/master/runtime/doc/help.txt)
- [Vim quick reference](https://github.com/vim/vim/blob/master/runtime/doc/quickref.txt)
- [Vim command index](https://github.com/vim/vim/blob/master/runtime/doc/index.txt)
- [Options](https://github.com/vim/vim/blob/master/runtime/doc/options.txt)
- [Mappings, abbreviations, and user commands](https://github.com/vim/vim/blob/master/runtime/doc/map.txt)
- [Repeating commands and macros](https://github.com/vim/vim/blob/master/runtime/doc/repeat.txt)
- [Files and writing](https://github.com/vim/vim/blob/master/runtime/doc/editing.txt)
- [Windows, buffers, and tab pages](https://github.com/vim/vim/blob/master/runtime/doc/windows.txt)
- [gVim GUI](https://github.com/vim/vim/blob/master/runtime/doc/gui.txt)
- [Diff mode](https://github.com/vim/vim/blob/master/runtime/doc/diff.txt)
- [Terminal, jobs, and channels](https://github.com/vim/vim/blob/master/runtime/doc/terminal.txt)
- [Syntax highlighting](https://github.com/vim/vim/blob/master/runtime/doc/syntax.txt)
- [Tags](https://github.com/vim/vim/blob/master/runtime/doc/tagsrch.txt)
- [Quickfix](https://github.com/vim/vim/blob/master/runtime/doc/quickfix.txt)

## Implementation audit (September 2026)

This inventory was checked against the actual implementation. It is a candidate
list, not a second product specification. All TODO3 requirements were adopted.
The new Regex v1 requirements in AGENTS.md were also implemented. The selected
capability/option dispositions are machine-readable in
[docs/compatibility.json](docs/compatibility.json); the executable command matrix
and exact semantic fixtures remain in `tests/vim_command_matrix.rs`,
`tests/vim_exact_conformance.rs`, and `tests/vim_deep_semantics.rs`.

- **Settings and styles:** versioned `~/.evim/config.json`, validated atomic
  writes, migration, unknown-key retention, the four format default files,
  native Style menu, and inherited-vs-source cascade are implemented. Settings,
  themes, status preferences, compound `:set` validation, Boolean operations,
  and ordered `fileformats` operations already existed in part; this audit does
  not treat them as wholly missing.
- **Search:** `ignorecase`/`ic`, `smartcase`/`sc`, and `wrapscan`/`ws` are now
  buffer-shared switches. Regex v1 supports Unicode captures, named captures,
  multiline matches, semantic line/document assertions, typed replacement
  breaks and retained capture styling. Unsupported atoms, resource limits,
  cancellation, stale results and grapheme-splitting edits have explicit errors.
  Selected edits validate atomically, including histories and undo state.
- **File and native interaction:** `:e` replaces the active pane; `:E` opens a
  new window. `:pwd`, `:cd`/`:chdir`, `:update`, native alternate/ranged writes,
  overwrite guards and external-file checks are connected to the host. Native
  prompt selection/clipboard editing, selectable output with a close button,
  context-menu access and system-directed wheel scrolling are implemented.
- **Conversion:** HTML/Markdown conversion translates formatted semantics and
  styling; unsupported information produces a persistent warning. Explicit
  Latin-1 conversion reports substituted characters. Code and Code Block use
  real system monospace resolution. Plain-text and same-family projection
  switches continue to preserve source bytes.
- **Already present:** counted/nested macros, uppercase macro append, normalized
  event identity, macro recursion/work limits, semantic dot repeat,
  `:normal!`, substitute repeats, local marks and jumps, typed special registers,
  branching undo and time navigation, shared document identity, recovery journals,
  readonly write guards, exact source/ranged save preparation, save-race handling,
  styled lists, paragraph controls, multi-click selection and native clipboard
  integration have behavioral tests. Their broader Vim variants below are not
  implied by those claims.

## Skipped compatibility candidates

No TODO3 item was skipped. The following agent-generated GAPS candidates are
not adopted for the current product. This is a scope decision, not a claim that
all of Vim's corresponding features are implemented.

- **A second Vim settings/configuration universe:** arbitrary option types and
  hundreds of Vim aliases, `:setglobal`/global-local inheritance emulation,
  `:verbose set`, terminal input timeouts, Vim startup scripts and hot reload
  are skipped. The adopted typed settings, native Settings UI, buffer search
  policy and per-view layout controls have explicit ownership; unsupported
  options fail rather than accepting inert values.
- **Mappings, abbreviations and general automation:** recursive/remappable input,
  `<Leader>`/`<Plug>`, expression registers/mappings, custom operators, user Ex
  commands, autocommands, Vimscript/Vim9 and plugin runtimes remain the explicit
  non-goals in AGENTS.md. Existing normalized macros and `:normal!` remain useful
  writing automation without promising a scripting environment. Last-Ex/register
  execution aliases and editable event-notation imports are not separately
  adopted in this pass.
- **Hidden buffers and argument-list lifecycle:** Vim listed/unlisted/unloaded/
  wiped buffers, `:b*`, alternate-file state, `:arg*`, file-iteration commands,
  Vim tab pages and their local directories are skipped. Native documents,
  windows and stacked views already own that lifecycle. Only the application
  working directory is adopted; `:lcd`, `:tcd`, previous-directory history,
  shell/glob/environment filename syntax, completion, `++opt` and `+cmd` require
  a separate product decision. Additional `:read`/`:file` and `ZZ`/`ZQ` aliases
  are not claimed; native open/save/close and the adopted Ex equivalents remain
  available.
- **Broad Ex batch language:** pipelines, nested commands, scripting modifiers,
  `:global`/`:vglobal`, `:sort`, `:retab`, `:execute`, command-line windows and
  file iteration are skipped. Reordering formatted source with hidden syntax,
  synthetic blocks and style provenance needs defined semantic intentions;
  flat Vim line scripts are not automatically safe equivalents. Native prompt
  editing/output and the explicitly tested Ex grammar remain the contract.
- **Regex compatibility beyond canonical v1:** Vim regex modes, search offsets,
  expression replacement, replacement case-conversion escapes, previous-string
  expansion, and persistent Vim search history are skipped. Canonical Regex v1
  deliberately rejects these spellings. Incremental search, persistent match
  highlighting and count/confirmation interfaces remain future presentation
  capabilities; the current matcher and structured confirmation preview do not
  pretend to provide those interfaces.
- **Terminal-cell editing and source-code commands:** fixed-column `textwidth`/
  `wrapmargin` reflow, `gq`/`gw` hard wrapping, `formatoptions`/`comments`, virtual
  cells/block padding, Virtual Replace, ROT13/numeric tooling, method/bracket
  navigation and source-tag text objects are skipped. eVim uses proportional
  visual rows, shaped graphemes, semantic paragraphs, lists, layout indentation
  and style controls. Soft wrapping must not create persisted newlines merely
  to imitate terminal layout.
- **Parallel selection/session state:** Vim Select mode, global/numbered file
  marks, cross-buffer change/jump lists, session/viminfo history and expression/
  alternate-file registers are skipped. Core Visual modes, native selection,
  current-buffer anchors, native document windows and branch-aware undo define
  the current editing model. Platform primary-selection distinctions are left
  to future frontends; macOS uses its native pasteboard.
- **Historical file-policy emulation:** Vim swap serialization, automatic
  autoread, backup/writebackup option families, permission escalation,
  `:view`/nonmodifiable mode, hard-link replacement and full metadata/xattr
  policy emulation are skipped. Native recovery ownership, explicit reload,
  external-change warnings, overwrite checks, readonly policy and exact source
  saves are retained; automatic reload never silently discards writing.
- **Compatibility-only chrome:** terminal number/whitespace/cursor-column
  gutters, Vim display-column options, a gVim toolbar and exhaustive GUI aliases
  are skipped. The current native typography, status controls and accessibility
  surface remain the supported UI. This does not claim exhaustive accessibility
  parity or preclude a separately designed prose navigation feature.
- **Programming, processes and remote execution:** everything in the section
  explicitly marked DO NOT IMPLEMENT remains skipped, as do shells/filters,
  jobs/channels, terminal buffers, build/quickfix/LSP/tag tooling, remote Ex and
  server scripting. They do not fit the current human-language editor and would
  introduce a separate execution/security model. Native open-file routing and
  recovery ownership cover the current multiple-instance requirements.

The compatibility JSON is deliberately scoped to the adopted eVim surface and
these feature families. Generating an exhaustive entry for every unadopted Vim
key, option, abbreviation and historical alias is also skipped; it would imply
an unselected compatibility roadmap rather than improve the current editor.
