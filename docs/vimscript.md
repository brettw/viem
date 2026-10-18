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
Interactive function definition is unsupported; define functions in sourced or
startup scripts. Input/confirmation compatibility functions return defaults
instead of opening prompts, and `strftime()` formats dates in UTC only.

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
New input also cancels it. Each native boundary finishes the current undo
segment; no mutable document transaction stays open across a native callback.
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
Pre-save errors prevent the save; post-save edits keep `:wq`/`:xit` documents
open and dirty even though the earlier bytes were saved successfully.
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
| Fixed-value compatibility | Options with a fixed Viem behavior are accepted silently at the matching value; contradicting changes produce a notice. Examples include `nocompatible`, UTF-8 internal `encoding`, `linebreak`, `hidden` and fixed numeric-format behavior. |
| Inert compatibility | Terminal, plugin and display-tuning options such as `termguicolors`, `ttimeout`, `runtimepath`, `loadplugins` and `synmaxcol` are accepted silently without enabling those mechanisms or changing Viem's resource limits. |
| Ignored | Backup, swap and undo-file options are accepted silently and do not replace Viem's existing recovery/history policies. |
| Recognized unsupported | Shadow values can be read, but a nondefault change produces a notice. Examples include `guicursor`, cursor/relative-number overlays, statusline customization, folds and spelling. Unknown names report E518. |

`exists('&name')` tests whether an option is recognized; `exists('+name')` tests
whether Viem reports working support. Neither implies complete Vim semantics for
an application mapping. `iskeyword` affects editing motions, text objects,
abbreviations and word deletion. Search keyword extraction and Unicode regex
boundaries retain their independent documented behavior.

The canonical names, abbreviations, types, scopes, defaults and treatment lists
are in [`command/options.rs`](../src/core/command/options.rs), based on Vim 9.1's
option table. Machine-specific defaults are normalized to portable home/temp
patterns. `sc` abbreviates `showcmd`; `smartcase` uses `scs`. `linebreak` is a
fixed matching value, so `nolinebreak` produces a notice.

Mapped presentation settings are transient application configuration; they do
not rewrite the user's settings or theme files. Source/model options remain
separate verified document transactions.

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

## Implementation boundaries

The language engine in [`src/core/vimscript/`](../src/core/vimscript/) does not
depend on the command interpreter or coordinator. `ScriptHost` connects it to
editor commands, options, registers, editor builtins, capability queries,
messages, dynamic variables such as `v:count` and `b:changedtick`, file reading
and suspension.

| Area | Responsibility |
| --- | --- |
| `value.rs`, `error.rs`, `vvars.rs` | Byte strings (`VStr`), scalar/container values, conversions and display, Vim errors/exceptions, and writable versus read-only `v:` variables. |
| `expr.rs` | Evaluate while parsing, including skipped branches, subscripts, method calls, lambdas, interpolated strings and curly-brace names. |
| `exec.rs`, `state.rs` | Runtime frames and unwinding, host interface, per-buffer script state, functions, closures and per-view `w:` state. |
| `commands.rs`, `excmds.rs` | Command resolution, modifiers, ranges and command-specific bar/comment rules; the catalogue is based on Vim 9.1. |
| `let_cmd.rs`, `function_cmd.rs`, `user_commands.rs`, `autocmd.rs` | Assignments and locking, heredocs, function/user-command definitions, autocmd groups and file patterns. |
| `keys.rs`, `source.rs` | Key notation and script identities, bounded source reading, continuation lines, path expansion and filename modifiers. |
| `builtins/`, `printf.rs`, `regex.rs` | Builtin operations, Vim formatting and cached, fuel-metered `VimPattern` matching for strings. |
| `tests.rs` | In-memory host, recorded-oracle fixtures and the ad-hoc script runner. |

Evaluation while parsing is deliberate: runtime types decide whether `dict.key`
is member access or `.` is concatenation, and can make `x .. 2.5` parse as
`x .. 2 . 5`. Preserve this behavior when changing expression parsing.

Statement-level user-function calls, `execute`, `source`, autocmds and user
commands push frames on one explicit runtime stack. `ScriptHost::should_suspend`
lets `Runtime::run` yield between commands. Calls inside expressions use
synchronous `run_nested`; they cannot suspend while the nested depth is nonzero.
Non-language commands reach `ScriptHost::execute` as a cleaned `HostCommand`
(range, name, bang and arguments, without modifiers, bars or comments). The
engine expands `<Leader>`, `<LocalLeader>` and `<SID>` for mappings and
abbreviations before handing them off.

Editor integration lives in
[`command/scripting.rs`](../src/core/command/scripting.rs),
[`options.rs`](../src/core/command/options.rs),
[`abbreviations.rs`](../src/core/command/abbreviations.rs),
[`mappings.rs`](../src/core/command/mappings.rs),
[`coordinator/scripting.rs`](../src/core/coordinator/scripting.rs) and
[`autocmds.rs`](../src/core/coordinator/autocmds.rs), with the narrow C ABI and
native frontends handling external effects. `ReplayFrame::Script` participates
in the coordinator's ordinary compound replay, publication and undo machinery.
Both frontends submit source through `viem_core_view_execute_command_line`;
there is no native line-by-line source executor.

Continuation invariants:

- Suspension retains the exact frame stack and remaining fuel/command budget.
  A single-use monotonic token is the last effect in its batch. The host applies
  effects in order and acknowledges through `viem_core_view_resume_script`.
- Stale tokens cannot resume or cancel a newer run. Native handlers validate
  the originating document/view and continuation-marker placement. Messages
  preceding a native action are published before that action.
- Cancellation drops later commands and restores the completed command's mode
  and undo state, including a temporary Insert/Replace Ctrl-O command. It never
  replays already completed edits or external effects.
- macOS sessions borrow their document backend. Native completions and queued
  acknowledgements must retain that owner until they finish, including after
  the surface closes, then release it. Teardown is covered by a native test.

The engine splits multi-argument `:set` commands before invoking the atomic
single-command executor. `let &opt = value` uses `options::set_command_for`;
reads combine actual editing values with compatibility shadow values through
`options::read_option`. Reads of encoding/BOM policy resynchronize after undo.
Invalid encoding, font and continuation-indent requests fail before publishing
shadow values. Startup has a theme catalogue before evaluation, drains
application effects after view attachment, and marks them for first-buffer-only
application without suppressing later buffers' events.

Syntax-file and imported color-scheme evaluation still use the separate bounded
evaluator in [`document/syntax/vim/loader/`](../src/core/document/syntax/vim/loader/).
Sharing it with this language engine requires a determinism and caching review;
the common editor script entry points do not imply that these evaluators merged.

## Oracle compatibility notes

The recorded development oracle was Vim 9.1.1752 (`/usr/bin/vim` on the macOS
validation host). That is fixture provenance, not a change to the product's
Vim 9.2 compatibility target. Relevant Vim help files are `options.txt`,
`builtin.txt`, `eval.txt`, `autocmd.txt`, `map.txt`, `cmdline.txt` and `index.txt`.
The following behaviors are pinned by the engine fixtures; preserve them when
refactoring rather than substituting more familiar language semantics.

- Errors in a script are reported and execution continues with the **next
  line**; the rest of the failing line (after `|`) is skipped. Error context
  is the line of the failing command.
- A non-`abort` function keeps running after an internal error, and the
  caller is unaffected (its line continues). An `abort` function (and every
  **lambda**) returns **-1** to the caller's expression at the error; the
  calling command completes, then the rest of the caller's line is skipped,
  and an `abort` caller aborts too. Implemented with `Runtime::did_emsg`
  (reset per command; `call_user_function` restores the caller's value OR
  "aborted").
- Inside `:try`, errors anywhere (body, `:catch`, `:finally`) become
  exceptions `Vim(cmd):E123: msg`. An **uncaught** exception aborts the whole
  run (all frames); a thrown value reports `E605: Exception not caught: x`, an
  error-exception reports the original error text.
- `:silent!`: errors set `v:errmsg`, are not reported, do not become
  exceptions and do not abort `abort` functions. They do **not** skip the
  rest of the line by themselves (did_emsg stays false): `silent! set bad |
  echo 1` echoes. The command's own next-command rule still applies: a
  failed `:call` (callee or argument error) skips the rest of the line
  outside `:try`; an unknown command (E492) skips it; after a failed
  expression in `:let`/`:echo`, a next command exists only if `|` directly
  follows the failure point (`commands::next_after_failure`). The error is
  dropped while the modifier is active (`commands::silence_error`), and
  silence stays in effect inside frames the command pushed
  (`Runtime::transfer_silence`). Pinned by fixture `silent_errors`.
- Map commands (no trailing comments, Vim's EX_NOTRLCOM) keep trailing white
  space, also before `|` (`commands::split_bar` trims only when comments are
  allowed).
- `"` starts a comment after `:set` (Vim gives E525 for `set com=:"`).
- `:throw` converts its value to a String; a List reports E730 directly even
  inside `:try`. Values starting with "Vim" give E608.
- Unwinding (`break`, `continue`, `return`, exceptions) **deactivates** inner
  conditionals instead of removing them; their skipped `endif`/`endtry` pop
  them. A `:finally` runs; its pending action resumes at `:endtry`; an
  exception inside `:finally` discards the pending action.
- Trailing `"` comments are allowed after `let`, `const`, `if`, `elseif`,
  `endif`, `while`, `for`, `function`, `return`, `call`, `unlet`, `try`,
  `throw`, `catch`, `eval`; **not** after `echo`, `echon`, `execute`
  (`"` starts a string). Comment/bar exception lists are in `excmds.rs`
  (`:help :bar`, `:help :comment`). In map commands `\|` is a literal bar.
- Errors Vim reports through `eap->errmsg` end with `: {command}`, e.g.
  `E588: :endwhile without :while: endwhile`,
  `E488: Trailing characters: extra: NoArgs extra` (user command arg checks).
- `:echo` prints each argument as soon as it is evaluated (first as a new
  line, later ones as `" " + text` continuing the line), so errors from a later
  argument appear in between. `:echomsg`/`:echoerr` join all arguments first.
- Vim prints "line N:" for an error only when (file, function, line) changed
  since the previous error (`msg_source`); the test host emulates this.
- `:doautocmd` with no matching autocommands echoes
  `No matching autocommands: {args}`.
- Unprefixed names in a function: `l:` first, then compat names (`count` ->
  `v:count`, also `errmsg`, `shell_error`, `this_session`, `version`), then
  closure scopes. Assigning an existing closure variable updates the outer
  scope; assigning `count` gives `E46: Cannot change read-only variable
  "count"`.
- Lambda arguments are **locals** (`l:x`); `a:` holds only `0`, `000`,
  `firstline`, `lastline`. `{x -> a:x}` is E121.
- `[` after a function call (by name, `->method()`, or `(...)` subscript)
  may follow white space: `toupper("ab") [1]` is "B"; `[1,2] [0]` is two values.
- List/Blob slices whose start is before the beginning are empty
  (`[1,2,3][-4:]` is `[]`); string slices clamp (`"abc"[-5:]` is "abc").
- `{expr}` alone is a curly-brace name, not a Dictionary (`echo {name}_1`).
- Floats print with Vim's own `%g`: fixed for 0.001 <= |f| < 1e7 else
  exponent, 6 digits, trailing zeros removed keeping `.0` (`1.0e-4`,
  `1.234568e8`, `100000.0`). `printf('%g')` uses the same rules; `%e` is C's.
  `%c` takes the low byte. `round()` rounds half away from zero.
- Number parsing: `"010"+0` is 8 (octal), `" 5"+0` is 0 (no white space skip),
  `str2nr(" 42")` skips white space; `1e3` is not a float literal.
- Division by zero: `5/0` = max, `-5/0` = -max, `0/0` = min, `5%0` = 0.
- `substitute()` follows `do_string_sub`: an empty match repeated at the
  previous empty match position copies one character; matching stops once a
  match reaches the end (`substitute("aaa","a*","x","g")` is "x").
- `split()` follows `f_split` (empty pieces only between items).
- A builtin's callback error (e.g. `map(l, function('F'))` with E118) is
  reported, the builtin stops and still returns its value; the command
  continues (`builtins::callback_failed`). Inside `:try` it propagates.
- `v:` read-only vs writable set comes from Vim's flags (`vvars.rs`):
  writable are char, errmsg, warningmsg, statusmsg, hlsearch, searchforward,
  oldfiles, swapchoice, swapcommand, fcs_choice, scrollstart, errors,
  this_session, testing, colornames, lnum.
- Dictionaries iterate in sorted key order (Vim uses hash order; fixtures
  avoid printing multi-key dicts unless sorted).

## Validation

Portable language fixtures and integration tests run without an installed Vim:

```sh
cargo test --lib vimscript::
cargo test --test all vimscript_
```

The fixtures live in [`tests/all/fixtures/vimscript/`](../tests/all/fixtures/vimscript/),
with `.vim` inputs and exact `.out` recordings. Select one fixture or run an
ad-hoc file through the in-memory host:

```sh
VIEM_VIMSCRIPT_FIXTURE=silent_errors cargo test --lib vimscript::
VIEM_DEBUG_SCRIPT=/path/example.vim cargo test --lib vimscript::tests::debug_script -- --ignored --nocapture
```

Refresh oracle output explicitly with
[`scripts/vimscript-oracle.sh`](../scripts/vimscript-oracle.sh):

```sh
scripts/vimscript-oracle.sh tests/all/fixtures/vimscript/silent_errors.vim
# Omit fixture arguments to refresh every recording.
# Set VIEM_VIM_ORACLE=/path/to/vim to choose the development oracle.
```

The recorder runs without user configuration in a UTF-8 locale and normalizes
path/context headers. Review the output diff; trailing spaces in `.out` files
can be intentional and must not be stripped. Fixtures avoid depending on Vim's
unordered dictionary iteration. Installed Vim is only a development oracle,
never a build/runtime dependency.

Fixtures cover expressions, variables, control flow, exceptions, functions,
builtins, execute, continuations, command/autocmd behavior, regex strings,
silenced errors and container/heredoc/partial behavior. Direct engine tests cover
nested source, script identities, `finish` and SourcePre/Post. Integration tests
under `tests/all/vimscript_*.rs` cover actual publication/undo, source and startup,
mappings/abbreviations, events, options and native continuation cancellation.
Resource tests include overflowing ranges, oversized repeat/printf, recursive
flattening, bounded glob traversal and bounded regular-file reads.

Native tests exercise ordered source/read effects, failed and cancelled host
actions, queued-continuation owner teardown/release, save callbacks, startup
application delivery, mapping timers and native option consumers. Run macOS
tests with `scripts/test-mac.sh` using its isolated configuration; see
[macOS development](macos-development.md). Windows native diagnostics must run
on Windows; C ABI and generated-binding checks alone do not validate WinUI
behavior. Windows execution was unavailable during the completion validation;
bindings and continuation dispatch received portable/static checks only.

For shared editing changes, follow the [staged validation workflow](development-workflow.md):
compare quick consistency with a clean baseline before the full suites and keep
manifests/logs in ignored `target/validation/` output. The completion run's Rust
logs are in `target/validation/vimscript-completion/`; its final macOS logs are in
`target/validation/vimscript-native-final/`. These are local run artifacts, not
required build inputs or a guarantee about later changes.

An unrelated baseline failure remained at completion on 2026-10-17:
`audit_format_regressions::repository_global_delete_preserves_completed_prefix_on_supported_targets`
reported `ExError(Execute(Document(VerificationFailed)))` in
[`tests/all/audit_format_regressions.rs`](../tests/all/audit_format_regressions.rs).
It also reproduced at clean commit `95adf3df6da5b00ab3f1cc8c4cd17218869e05f1`;
`baseline-known-failure.log` in the Rust run directory preserves that evidence.
Recheck the baseline when investigating it; do not treat this historical result
as permission to ignore a changed or new failure.
