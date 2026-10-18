# Vim script support

Viem supports a bounded legacy Vim script profile for configuration and editor
automation. `startup.viem`, `:source`, submitted `:` lines, mapping commands and
`:normal` share one engine. This is not a complete Vim runtime or a plugin host;
Vim9 script is unsupported and a `vim9script` declaration stops that script.

## Loading and state

The profile directory is `~/.viem` on macOS and `%USERPROFILE%\.viem` on Windows.
`VIEM_CONFIG_DIR` overrides it for settings, themes and `startup.viem`. Viem creates
an empty startup file when missing, reads one snapshot per profile launch, and
evaluates it for each buffer after JSON preferences. A file must be UTF-8, with an
optional BOM and LF or CRLF endings, and at most 1 MiB.

Startup permits configuration logic and definitions: options, mappings,
abbreviations, functions, commands and autocmds. Immediate editing, file/window
actions, encoding/BOM conversion and `fileformat`/`fileformats` assignments are
rejected. Definitions may perform supported actions when invoked later.
Application settings requested during startup apply only for the first buffer;
opening another document does not reset later interactive settings.

Viem does not automatically load `.vimrc`, `.gvimrc`, runtime directories or
plugins. A user may explicitly source a compatible file:

```vim
source ~/.vimrc
```

Unsupported commands in an existing vimrc can produce diagnostics. `has('viem')`
is true, so a shared configuration can guard Viem-specific choices:

```vim
let mapleader = ','
nnoremap <Leader>w <Cmd>write<CR>
inoreabbrev teh the

if has('viem')
  set timeoutlen=400
  set clipboard=unnamedplus
endif

function! TitleCaseLine() abort
  call setline('.', toupper(getline('.')))
endfunction
nnoremap <Leader>u <Cmd>call TitleCaseLine()<CR>
```

Script state belongs to the buffer, including `g:` variables, functions, script
locals, user commands, mappings and autocmds. Its views share that state; `w:`
variables are keyed by view. This differs from Vim's process-global `g:` state.
There is no general registry of script-accessible Vim buffers or tab pages.

## Language and editor operations

The engine includes legacy Numbers, Floats, byte Strings, Lists, Dictionaries,
Blobs, Funcrefs, partials and lambdas; scoped variables and option/register access;
`let`, `const`, `unlet`, locking; `if`, loops, functions and returns;
`try`/`catch`/`finally`/`throw`; `echo`, `execute`, `eval`, `call` and `source`.
It supports expression operators, indexing/slicing, method calls and closures.
Strings retain Vim's byte-oriented indexing; Unicode character functions remain
separate. Dictionaries iterate in sorted key order rather than Vim's hash order.

Builtins cover arithmetic, string/Unicode handling, Lists/Dictionaries/Blobs,
conversion, JSON, formatting, regular expressions, callbacks, assertions and
read-only environment/filesystem queries. Editor operations include `getline`,
`getbufline`, `line`, `col`, `getpos`, `cursor`, `setpos`, `setline`, `append`,
`deletebufline`, register access, search, history and mapping queries. Editing
uses the same verified document transactions as ordinary commands. Line-based
buffer builtins address physical source lines; they do not manufacture a second
formatted-document model.

The name catalogue also recognizes builtins without an implementation; calling
one reports an unsupported-function error. Do not treat every familiar name as
full Vim compatibility. Some buffer/window/tab introspection is a single-buffer
compatibility view: identifiers and window dimensions are placeholders, not a
query of native window geometry. There is no cross-buffer editing API.

Script regular-expression functions use the bounded legacy Vim pattern engine.
Ordinary editor search retains Viem's documented regex dialect; scripting does
not replace that search contract.

Filesystem reads such as `readfile`, `filereadable`, `glob` and `executable` are
available. Script builtins cannot write/delete/rename files, modify environment
variables, start processes/jobs/timers, or call foreign libraries. `mkdir()`
returns 0 without creating a directory. Explicit supported editor commands such
as `:write`, `:read` and `:edit` still use the normal native file workflow.
Shell commands/filters, `:source!`, terminal jobs and plugin/runtime loading are
unsupported.

## Errors, native effects and undo

An ordinary script error reports its location and continues at the next line,
skipping the remainder of the failing line. `abort` functions, exceptions,
`silent!` and `try` follow their legacy control-flow rules. An uncaught exception
or exhausted resource budget stops the run. Successful earlier edits remain.
Changes from one script action share a compound undo unit; explicit history
operations can synchronize that grouping.

Native file/window/application requests suspend a script at a command boundary.
The host completes the request before acknowledging the continuation. A failed
or cancelled operation stops the remainder; completed edits and external effects
are retained. A changed or closed owning core/view cancels the pending script.
Continuation into a newly opened document or a different selected pane is not
implemented.

Calls made inside an expression are synchronous. Native file/window commands in
such calls are rejected. Application-setting requests wait until the enclosing
command reaches a resumable boundary; script option reads already reflect the
accepted option value. Interactive substitute confirmation is unsupported inside
a script. Native dialogs remain explicit host operations.

Work is bounded: scripts are limited to 1 MiB, source nesting to 16, function
depth to 100, runtime frames to 200, expression depth to 1000 and runtime fuel to
20 million units. File-read builtins accept regular files of at most 8 MiB.
Globbing is limited to 10,000 results, 100,000 traversal steps/directory entries
and 128 nested path steps, including searches that find nothing. `globpath()`
shares those limits across its directories. Generated ranges, repeated lists and
flattened lists are limited to one million items; string/blob repetition and
`printf()` output to 64 MiB. `printf()` width/precision fields are limited to one
million, and flattening rejects cycles or more than 256 active list levels.
Individual script regex text is limited to 256 KiB. Core command replay has its
own finite budget. Recursive mappings and abbreviations cannot run indefinitely.

## Mappings, abbreviations and commands

Recursive/nonrecursive and mode-specific mapping forms, their command
abbreviations, listing and `maparg`/`mapcheck`/`hasmapto` queries are supported.
Attributes include `<silent>`, `<nowait>`, `<unique>`, `<special>`, `<buffer>`,
`<expr>` and `<script>`. `<Leader>`, `<LocalLeader>`, `<SID>`, `<SNR>`, `<Plug>` and
`<Cmd>…<CR>` use the script/key-notation path. Trailing spaces in replacements
remain meaningful, and `\|` preserves a literal bar in a mapping command.
Expression mappings cannot edit text or switch windows while resolving input.

All mappings are buffer-owned; `<buffer>` remains queryable compatibility
metadata. `lmap` and terminal mappings can be defined and listed but are dormant:
Viem has no corresponding language-input/terminal mode. F8 and Shift-F10 retain
native actions. `timeoutlen` and `timeout` govern ambiguous key-prefix waits.

Insert/Replace and command-line abbreviations support their `nore`, listing,
remove and clear forms, including `<expr>` and `<buffer>`. Expansion follows the
configured keyword class and a typed terminating character, Escape, Enter or
Ctrl-]. Literal-next input bypasses expansion; expansion shares the initiating
edit's transaction/undo behavior.

`:command` supports user-command definitions with argument, bang, range/count,
register, buffer and completion metadata and the corresponding replacement
placeholders. Native completion does not implement every Vim completion source.

## Autocommands

The profile supports groups, definitions/removal/listing, file patterns,
`<buffer>`, `++once`, `++nested`, `:doautocmd`, `:noautocmd` and event context such
as `<afile>`, `<amatch>`, `<abuf>` and `v:event` where applicable.

`BufNewFile`/`BufReadPost` fire when the first view attaches; `BufEnter` and
`BufWinEnter` fire for each attachment. Native hosts fire `VimEnter`/`GUIEnter`
once, focus events and full-buffer save pre/post events. Native save handlers
finish synchronously: `BufWritePre` precedes byte capture, and `BufWritePost`
precedes the final dirty-state check for write-and-quit. Core edits and
core-only options are allowed; native file/window operations and application
settings fail with E565 before publishing their option or theme state. This
restriction includes commands reached through functions, mappings and `:normal`.
The same restriction applies to native callbacks received while another script
awaits a native operation. `FileType` follows
`:set`, option assignment and native language changes. Insert entry/leave events
observe actual per-input transitions, including mappings and macros. Source
entry/exit and color-scheme changes also have hooks.

`TextYankPost` follows yanks, excluding the black-hole register; delete/change
notifications remain unsupported. Its read-only `v:event` includes `regcontents`,
and handlers cannot edit text. `InsertLeavePre` currently runs immediately after
the input has changed mode, alongside `InsertLeave`; `mode()` already reports
Normal. Ranged/alternate-file `FileWritePre`/`FileWritePost` hooks are unsupported.
The recognized event list lives in [`autocmd.rs`](../src/core/vimscript/autocmd.rs);
other names may be retained without automatic firing. Definitions remain per
buffer. Nested automatic execution requires `++nested` and stops at depth ten.

## Options and native settings

`:set`, `:setlocal`, `:setglobal` and `let &option = value` share option handling.
A multi-option script command applies arguments left to right and stops at its
first error, retaining earlier accepted changes. Existing editing options retain
their established scope; compatibility shadow values are per buffer, rather than
a full Vim process-global/default-window model.

| Category | Behavior |
| --- | --- |
| Implemented editing options | Existing wrapping, line numbers, search, indentation, whitespace and formatting options, plus `clipboard`, `gdefault`, `shiftround`, `joinspaces`, `whichwrap`, `startofline`, `history`, `iskeyword` and language overrides. |
| Source/model options | `readonly` changes read-only state. `fileencoding` and `bomb` use verified encoding transactions and ordinary undo; startup forbids source conversion. |
| Fixed-value compatibility | Terminal/UI assumptions are accepted when compatible; contradicting changes produce a notice. Examples include `nocompatible`, UTF-8 internal `encoding`, `linebreak`, `termguicolors` and fixed numeric-format behavior. |
| Ignored | Backup, swap and undo-file options are accepted silently and do not replace Viem's existing recovery/history policies. |
| Recognized unsupported | Shadow values can be read, but a nondefault change produces a notice. Examples include `guicursor`, cursor/relative-number overlays, statusline customization, folds and spelling. Unknown names report E518. |

`exists('&name')` tests whether an option is recognized; `exists('+name')` tests
whether Viem reports working support. Neither implies complete Vim semantics for
an application mapping. `iskeyword` affects editing motions, text objects,
abbreviations and word deletion. Search keyword extraction and Unicode regex
boundaries retain their independent documented behavior.

| Vim setting | Native mapping and limits |
| --- | --- |
| `colorscheme` | Case-insensitive match against available Viem theme names, with E185 for an absent name. `Default` selects the default theme. |
| `background` | `light` selects `Paper`; `dark` selects `Midnight`. The named theme must exist. Requests run in script order. |
| `timeout`, `timeoutlen` | Native mapping-prefix timer, milliseconds; default 1000. `notimeout` waits for more input. |
| `belloff`, `visualbell`, `errorbells` | Suppress native editor beeps. A visual bell does not flash the editor. Windows currently has no editor beep to suppress. |
| `guifont` | Shared Code font in memory. Comma-separated family names; underscores denote spaces; `:h<size>`, `:b`, `:i` are accepted. Size is 1–1000. Other font attributes are rejected. |
| `linespace` | Shared Code minimum line height in memory: 1.2 times the font size plus 0–1000 pixels. Zero selects normal spacing. It is not a font-rasterizer pixel-spacing setting. |
| `breakindent`, `breakindentopt` | Code's extra continuation margin. `breakindentopt` accepts empty or `shift:0` through `shift:1024`; other forms are rejected. Disabling removes the extra margin but retains original source indentation. |
| `scrolloff` | Top/bottom reveal margins measured in default line heights, capped at 1000 pixels. This is Viem margin geometry, not exact Vim screen-row behavior. |
| `laststatus` | Zero hides the status bar; 1–3 show it. Command input/output and required interactions can temporarily show a hidden bar. |
| `lines`, `columns` | Native window dimensions in default line/column units, subject to native minimum/maximum size. They do not impose a fixed text grid. |
| `guioptions` | `m` controls the Windows menu bar; `a` controls automatic selection. macOS retains its system application menu. Other flags have no mapped presentation. |

## Validation

Portable language fixtures and integration tests run without an installed Vim:

```sh
cargo test --lib vimscript::
cargo test --test all vimscript_
```

Recorded oracle outputs can be refreshed explicitly with
`scripts/vimscript-oracle.sh`; this maintenance step uses an installed Vim and is
not a build/runtime dependency. Native startup, effects, mapping-timer and
application-option tests exercise the actual bridge. Windows native diagnostics
must run on Windows; C ABI and generated-binding checks alone do not validate
WinUI behavior.
