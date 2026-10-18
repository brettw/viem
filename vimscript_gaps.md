# Vim script support: design, state and handoff

This file is the working handoff for adding legacy Vim script ("Vim 8
script") to Viem. It records the user's decisions, the design, what exists,
what was learned about Vim's behavior, how to test, and what is left. Keep it
current while working; it must let someone resume after an interruption.

Last updated: 2026-10-17. Scoped implementation and available validation are
complete. Rust and macOS results are recorded below, including the pre-existing
Markdown test failure and unavailable Windows execution. This file supersedes
the interrupted agent's pending-work list. Nothing has been committed.

The completed integration includes the core option consumers, abbreviations and
mapping edges, automatic events, native application/startup effects, and ordered
native script continuations. See §7 for the implementation and §8 for deliberate
profile limits. Current validation state is recorded in §5; do not mistake
compile-only or portable checks for a Windows native test run.

## 1. Request and decisions

The user asked to implement legacy Vim script support, used consistently for
every way Viem loads script text, with a test suite and "full Vim script
compatibility to the extent the commands are supported". Decisions given:

| Topic | Decision |
| --- | --- |
| Dialect | Legacy Vim script (Vim 8 / Vim 9 legacy). Vim9 script is out of scope: a `vim9script` file stops with a diagnostic. |
| Loading | Do **not** auto-load `~/.vimrc`/`~/.gvimrc`. Only compatibility: users run them with `:source` or from `startup.viem`. |
| One engine | `startup.viem`, `:source`, and every `:` command line (typed, from mappings, `:normal`) go through the same engine. Remove the native `:source` line splitting in both frontends. |
| Functions | "Match what Vim does": user functions run later (mappings, autocmds, user commands, `:` prompt), with buffer builtins (`getline`, `setline`, `line('.')`, `cursor`, `search`, ...). |
| Tier 2 | Implement the autocmd subset, `:command` user commands, and abbreviations (AGENTS.md previously excluded abbreviations; update it). |
| Options | Do the plan below ("Options plan"): support/no-op/mapped categories, recognize the rest. Map `colorscheme` to Viem theme selection by theme-name match. Do **not** implement `guicursor` (recognize only). Ignore undo and backup options silently. Implement only the cheap core-only feature options (list in §6). |
| Mappings | Implement the mapping gaps (attributes, `<Leader>`, abbreviated command names, `lmap`, key notation, `<expr>`, `<Cmd>`, listing, `\|` handling). |
| Commits | Do not commit unless asked. Another agent may work in the repo; keep edits scoped. |

AGENTS.md and the platform guides now describe the bounded legacy profile and
configuration-only startup execution. A complete Vim runtime remains out of scope.

## 2. Background: Vim script and vimrc facts

- A vimrc is Vim script: every line is an Ex command; `let`, `if`, `while`,
  `for`, `try`, `function` are Ex commands too.
- Two dialects: legacy (additive from Vim 7 to 9.2; nearly all vimrcs) and
  Vim9 script (opt-in by a first `vim9script` command; different syntax,
  stops at the first error).
- Vim's user files (first found wins): Unix/macOS `~/.vimrc`,
  `~/.vim/vimrc`, `$XDG_CONFIG_HOME/vim/vimrc` (9.2); Windows
  `$HOME\_vimrc`, `$HOME\vimfiles\vimrc`, `$VIM\_vimrc`. gvimrc equivalents
  are read after the vimrc. `defaults.vim` loads only without a user vimrc.
  Viem will not search these (decision above).
- Real vimrcs use: `set` lines, `syntax on`, `filetype plugin indent on`,
  `let mapleader`, `nnoremap <silent> <leader>...`, `colorscheme`,
  `if has(...)`/`exists('+opt')` guards, `augroup`/`autocmd`, `command!`,
  abbreviations, functions called from mappings, plugin-manager lines
  (`Plug`, `call plug#begin()`), `scriptencoding utf-8`.

## 3. Architecture

```
src/core/vimscript/        neutral engine (no dependency on command/coordinator)
  value.rs    Value, VStr (byte strings), List/Dict/Blob (Arc<Mutex>), Funcref,
              conversions, echo/string() rendering, Vim %g floats, comparisons
  error.rs    VimError (E-number + text), Exception, Fail {Error, Aborted,
              Exception, Interrupt}
  expr.rs     single-pass evaluate-while-parsing expression evaluator (Vim style,
              `evaluate=false` skip mode), subscripts, method calls, lambdas,
              interpolated strings, curly-brace names
  exec.rs     Runtime: explicit frame stack (Script, Function, Execute,
              Autocmd, UserCommand, CommandLine), conditional stack per frame,
              unwinding (break/continue/return/finish/exceptions/abort),
              did_emsg, variables & scopes, function calls, ScriptHost trait
  commands.rs do_one_cmd equivalent: modifiers, ranges, name resolution, `|` and
              comment splitting per command, all script commands
  let_cmd.rs  :let/:const/:unlet/:lockvar (targets, unpacking, heredoc, &opt, @r, $ENV)
  function_cmd.rs  :function / :delfunction (body collection, defaults, attrs, dict fns)
  user_commands.rs :command definitions + replacement expansion
  autocmd.rs  groups, events (all Vim event names), file-pattern globbing
  excmds.rs   Vim 9.1 Ex command table (name, min abbreviation) + bar/comment rules
  keys.rs     key notation ("\<C-W>", <Plug>, <SID>, <Cmd>, modifiers) encoded in
              strings with private-use code points; decode/render
  regex.rs    Vim regex on strings via document::syntax::vim::VimPattern (cached)
  printf.rs   printf()
  source.rs   script file reading (bounded, UTF-8, BOM, CRLF), continuation
              lines, path expansion (~, $VAR, %, <sfile>, modifiers), fnamemodify
  state.rs    ScriptState (Arc, per buffer): g:, v:, b:, t:, w: (per view id),
              script entries (s:), functions, user commands, autocmds, messages
  vvars.rs    v: variables and the writable set
  builtins/   text.rs, collections.rs, script.rs (exists/has/function/call/
              execute/eval/assert_*), system.rs (env/files/time; read-only)
  tests.rs    MemoryHost, fixture runner, debug runner
```

Design rules (keep them):

- **Strings are byte strings** (`VStr`), as in Vim: `"héllo"[1]` is one byte,
  `strlen` counts bytes.
- **Evaluate while parsing** (Vim's approach). Needed for runtime-type
  decisions such as `dict.key` vs `.` concatenation and `x .. 2.5` parsing as
  `x .. 2 . 5`.
- **Resumable frames.** Statement-level work (`:call F()` of a user function,
  `:execute`, `:source`, autocmds, user commands) pushes frames on one explicit
  stack, so a host can suspend between commands (`ScriptHost::should_suspend`)
  and `Runtime::run` resumes later. Calls made from inside an expression run
  synchronously (`run_nested`); suspension is impossible while `nested > 0`.
- **Host boundary.** The engine implements the language and script commands;
  every other command is a `HostCommand` (cleaned text: range+name+bang+args,
  no modifiers/bars/comments) given to `ScriptHost::execute`. Mapping and
  abbreviation commands get `<Leader>`, `<LocalLeader>`, `<SID>` expanded by
  the engine first. Options (`&opt`), registers (`@r`), editor builtins, `has()`
  overrides, command existence, messages, dynamic variables (`v:count`,
  `b:changedtick`), file reading and suspension are host methods.
- **Per-buffer state.** `ScriptState` lives with the buffer's command state
  (like registers and mappings in Viem). `startup.viem` runs once per buffer
  core. `w:` variables are keyed by view id. Deliberate difference from Vim's
  process-global `g:`; document it in AGENTS.md.
- **Bounded.** Fuel per Runtime (`DEFAULT_FUEL` 20M units), max frames 200,
  `maxfuncdepth` 100, source nesting 16, expression depth 1000, script files
  1 MiB, regular-file reads 8 MiB, glob 10,000 results/100,000 traversal steps,
  regex via fuel-metered VimPattern (256 KiB text per match call). Generated
  range/repeat/flatten lists cap at one million items; repeat/printf output at
  64 MiB; printf fields at one million; flatten depth at 256, with cycle rejection.
- **Restricted filesystem builtins.** Builtins cannot write files, delete,
  rename, run processes, or change environment variables: `writefile`, `delete`,
  `rename`, `system`, `systemlist`, `job_start`, `libcall`, `timer_start`
  error; `mkdir()` returns 0 silently (so `silent! call mkdir(...)` guards stay
  quiet); `setenv`/`let $X=` error via host. Reads (`filereadable`, `glob`,
  `readfile`, `executable` PATH search) are allowed. Supported editor commands
  such as `:write` use the normal native host workflow.

Integration lives in `command/{scripting,options,abbreviations,mappings}.rs`,
`coordinator/{scripting,autocmds}.rs`, the narrow FFI and both native frontends.
Document encoding/BOM policy changes use verified model transactions.

## 4. Vim semantics pinned with the Vim 9.1 oracle

`/usr/bin/vim` (9.1.1752) with runtime docs in `/usr/share/vim/vim91/doc`
(`options.txt`, `builtin.txt`, `eval.txt`, `autocmd.txt`, `map.txt`,
`cmdline.txt`, `index.txt`) is the reference. Findings implemented:

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

## 5. Testing and current validation

- Engine fixtures: `tests/all/fixtures/vimscript/*.vim` and recorded Vim outputs
  in `*.out`. `cargo test --lib vimscript::` runs these plus unit tests. Filter a
  fixture with `VIEM_VIMSCRIPT_FIXTURE=name`. Refresh oracle output explicitly
  with `scripts/vimscript-oracle.sh [fixture.vim ...]`; installed Vim is only a
  development oracle, never a build/runtime dependency.
- Fixtures cover expressions, variables, control flow, exceptions, functions,
  builtins, errors, execute, continuations, commands/autocmds, regex strings,
  silent errors and advanced container/heredoc/partial behavior. Nested source,
  script identity, finish and SourcePre/Post also have direct engine tests.
- `cargo test --test all vimscript_` exercises publication and undo, startup,
  sourced functions, mappings, abbreviations, automatic event context, option
  consumers and native continuation/cancellation through Core.
- Native tests cover actual source/read ordering, failed/cancelled host actions,
  startup application delivery, mapping timers and application-option consumers.
  Windows diagnostic tests are added but need a Windows host to run.
- Resource regressions cover overflowing ranges, oversized repeat/printf,
  recursive flattening, bounded glob traversal and bounded regular-file reads.
- Ad-hoc engine runner: `VIEM_DEBUG_SCRIPT=/path/x.vim cargo test --lib
  vimscript::tests::debug_script -- --ignored --nocapture`.

**Final validation:** `target/validation/vimscript-completion/` contains the
Rust manifest and logs. Rust sources remained frozen after that run. The
subsequent native-only repairs passed the full macOS rerun, whose manifest and
logs are in `target/validation/vimscript-native-final/`. No implementation edits
followed that successful rerun.

| Check | Result |
| --- | --- |
| `cargo check --lib --tests --examples` | Passed, without warnings. |
| Quick consistency against HEAD 95adf3d | Passed: zero newly failing cases; 73 repaired; four changed diagnostics in existing failing cases. |
| `cargo test --lib` | 1,900 passed, two ignored. |
| `cargo test --test all` | 1,703 passed, one known baseline failure (below). All scoped Vimscript tests pass, including cancellation/undo and synchronous save guards. |
| Engine oracle | All 13 fixture pairs pass as part of the library suite. |
| C# binding generator `--check` | Verified 206 ABI functions. |
| macOS native suite / C ABI probe | Passed: 935 XCTest cases, four opt-in benchmarks skipped, zero failures; 48 Swift Testing tests passed. C header matches Rust ABI; bundled runtime verified; app build passed. |
| Windows native diagnostics | Not executable here: no Windows/.NET toolchain. |
| Whitespace | Clean except intentional trailing spaces in recorded `.out` oracle output. Existing CRLF files retained. |

The sole integration failure is
`audit_format_regressions::repository_global_delete_preserves_completed_prefix_on_supported_targets`,
with `ExError(Execute(Document(VerificationFailed)))` at
`tests/all/audit_format_regressions.rs:277`. This is the same failure reproduced
at clean HEAD 95adf3d by the earlier baseline run; its log is preserved as
`baseline-known-failure.log` alongside the final logs. It is not a Vimscript
regression and was left outside this scope.

Earlier full runs found old tests requiring whole-command `:set` atomicity or
one listing effect. They now verify Vim's accepted left-to-right argument
semantics and additional compatibility-option output. A default keyword
regression was repaired without changing the editor's documented search dialect.
All scoped Rust failures were addressed before the final Rust run. The macOS
run additionally exposed a queued continuation retaining a session after its
borrowed document owner had gone away. Native completion now explicitly retains
the owner through acknowledgement, with a teardown/release regression test.
Three older save tests now expect cancellation to reach the script while the UI
continues to suppress its message. All 17 focused native file tests and the
subsequent full macOS suite passed after the repair, including the new
teardown/release regression (`native-ex-lifetime-rerun.log` in the Rust run
directory and the final native logs). Windows continuation dispatch and owner
lifetimes also received a static review; native Windows execution remains
unvalidated.

## 6. Options plan

Implemented in `src/core/command/options.rs`. The Vim 9.1 table records names,
abbreviations, types, defaults and scopes. Machine-specific defaults are
normalized to portable home/temp patterns.

Categories:

1. **Supported** (existing `:set` handling in `ex_execute.rs`): wrap, number,
   hlsearch, incsearch, ignorecase, smartcase, wrapscan, textwidth,
   fileformat(s), formatoptions, comments, formatlistpat, autoindent, tabstop,
   shiftwidth, softtabstop, expandtab, smarttab, list, listchars, keymodel,
   selectmode, autoselect, continuecomments*.
2. **Accepted silently when the value matches Viem** (store in a per-buffer
   shadow store; a contradicting value gives a "not supported" notice):
   nocompatible, ttyfast, ttimeout, ttimeoutlen, t_Co/t_*, term,
   termencoding, termguicolors, lazyredraw, encoding=utf-8,
   nrformats (bin,hex), splitbelow/splitright (on), linebreak (on),
   conceallevel, synmaxcol, updatetime, mousemodel=popup, signcolumn, mouse.
3. **Ignored silently (user decision):** undofile, undodir, undolevels,
   undoreload, backup, writebackup, backupdir, backupcopy, backupext,
   backupskip, swapfile, directory, updatecount, patchmode.
4. **Mapped onto Viem settings:** `colorscheme`/`background` -> theme
   selection request by case-insensitive theme-name match (E185 when absent;
   needs the theme list from the host); `breakindent`/`breakindentopt` ->
   Code continuation margin; `scrolloff` -> view margins/reveal; `laststatus`
   -> status bar visibility; `timeoutlen`/`timeout` -> mapping timeout
   (native timer); `belloff`/`visualbell`/`errorbells` -> beep suppression;
   `guifont`/`linespace` -> Code font/line spacing (in memory); `lines`/
   `columns` -> window size; `guioptions` m/a -> Windows menu bar/autoselect;
   `readonly` -> `viem_core_set_read_only`; `fileencoding`/`bomb` ->
   `set_encoding_with_effects` (source-changing, not at startup). App-level
   ones travel as new "application request" effects; native applies them
   (startup requests only for the first document of the process).
5. **Implemented cheap core features:** `clipboard=unnamed,unnamedplus`,
   `gdefault`, `shiftround`, `joinspaces`, `whichwrap`, `startofline`,
   `history`, `iskeyword`, `timeoutlen`, and `filetype`/`syntax` as language
   overrides (also fires FileType).
6. **Recognized, unsupported** (shadow value + notice; `exists('&x')` 1,
   `exists('+x')` 0): everything else in Vim's table, including `guicursor`
   (do not implement), cursorline, colorcolumn, relativenumber, showbreak,
   statusline, cmdheight, hidden, autoread, wildmenu, spell*, fold*, ...
7. Unknown names: `E518: Unknown option: x`.

`:set a b c` executes argument by argument when run by the engine (Vim
stops at the first error, keeping earlier ones): `scripting::split_set_command`
splits before calling the existing executor; the Ex executor itself still
validates one `:set` command atomically. `let &opt = v` becomes a `:set`/`:setlocal`/
`:setglobal` command (`options::set_command_for`). `&opt` reads use
`options::read_option` (existing display values + shadow store).

Fixed: `ex_execute.rs` mapped `sc` to smartcase; Vim's `sc` is showcmd and
smartcase is `scs`. `linebreak` is now a Matching option (accepted; `nolinebreak` gives a notice);
the old rejection test is updated.

## 7. Completed integration

### Shared engine and document transactions

- Every script entry point uses the engine. The old native line-by-line
  `:source` implementations and FFI are removed; both frontends execute source
  through `viem_core_view_execute_command_line`. Ordinary script errors retain
  Vim's next-line continuation rules, with file/function/line diagnostics.
- `ReplayFrame::Script` participates in the coordinator's existing compound
  replay, publication and undo machinery. Editor builtins stage semantic actions
  against the current document; `:normal`, mappings and fed keys use replay
  frames. Buffer names and physical source-line queries use the bound document.
- Native File/Window/Application effects suspend the exact frame stack and its
  remaining fuel/command budget. A single-use monotonic token is returned last
  in the effect batch. The native host applies effects in order and calls
  `viem_core_view_resume_script` with success or cancellation. New input, changed
  active native surface or view closure abandons pending work; stale tokens
  cannot resume or cancel a newer run. Prior messages are emitted before the
  native action they precede. macOS acknowledgement callbacks retain the borrowed
  document owner until completion, including teardown/cancellation.
- A native boundary closes the current undo segment; no mutable transaction
  remains open across callbacks. Cancellation retains completed edits/external
  effects and discards later commands. Synchronous expression evaluation rejects
  native File/Window commands; application requests yield at the enclosing
  command boundary. A save callback cannot start a second suspended host action.
- `fileencoding` and `bomb` are verified source/model transactions with undo;
  readonly state uses the document policy. Option reads synchronize with actual
  document policy after history navigation. Invalid encodings and unsupported
  native font/indent grammar fail before shadow values are published.
- Startup uses restricted execution, rejecting immediate source changes,
  encoding/BOM/fileformat policy and file/window commands, including `let &...`.
  Application effects are drained after view attachment. Explicit startup flags
  let native hosts apply them only once per profile/process; buffer events remain
  deliverable for every buffer. Native theme catalogues exist before evaluation.

### Mappings, abbreviations and options

- Mapping command abbreviations, attributes, mode-specific `<unique>`, script/key
  notation, leaders, listing/query metadata, `<expr>`, `<Cmd>`, literal bars and
  native timeout consumers are integrated. `lmap`/`tmap` definitions are retained
  but dormant because Viem has no corresponding modes.
- Abbreviations implement Insert/Replace and command-line triggers, full/end/non
  keyword forms, expression evaluation, listing/removal, literal-next bypass and
  bounded replay. Trigger classification follows `iskeyword`.
- Feature consumers include clipboard unnamed aliases, `gdefault`, `shiftround`,
  `joinspaces`, `whichwrap`, `startofline`, bounded/deduplicated `history`,
  `iskeyword`, and syntax/filetype language overrides. `iskeyword` controls motions, text objects,
  deletion and abbreviations. Editor search retains the documented independent
  Unicode regex boundaries and fixed Unicode alphanumeric/underscore keyword
  extraction; no keyword override is installed into its regex cache. Native arrows and page motions retain the
  deliberate product rules in AGENTS.md.
- Mapped native options now drive timers, margins, status visibility, bell
  suppression, Code fonts/line spacing, continuation indent, window size,
  Windows menu visibility and automatic selection. Native values are transient
  configuration, not rewrites of the user's theme/settings files. Exact mapping
  limits and approximations are documented in `docs/vimscript.md`.

### Autocommands and native lifecycle

- Automatic hooks cover BufNewFile/BufReadPost, FileType, BufEnter/BufWinEnter,
  InsertEnter/InsertLeavePre/InsertLeave, TextYankPost, SourcePre/Post,
  ColorSchemePre/ColorScheme, native BufWritePre/Post, FocusGained/Lost and
  once-per-process VimEnter/GUIEnter.
- `++once`, bounded `++nested`, `noautocmd`, explicit `doautocmd`, file-pattern
  context and locked yank `v:event` are tested. Black-hole yanks do not fire the
  event; yank handlers cannot mutate text. Script and replay-generated input
  preserve event ordering and avoid duplicate outer events.
- Mode events execute at the completed input transaction boundary.
  InsertLeavePre therefore observes the completed mode change; it is not a
  hook inside an uncommitted mutation. This deliberate timing limit is explicit
  in the profile documentation.
- Saves fire BufWritePre before capturing bytes and BufWritePost only after
  successful acknowledgement. Pre-event errors stop saving. The synchronous pre/post-save callbacks reject
  native host/application commands with E565 before publishing their option or
  command state; ordinary core edits remain allowed. The same guard applies to
  native callbacks while another script waits for its host acknowledgement.
  Alternate/range writes do not impersonate BufWrite events. Post-save edits keep
  `:wq`/`:xit` documents open. Native cancellation propagates to the suspended script.
- C header, ABI probe and generated Windows bindings include theme/application
  effects, startup drain, event firing and continuation APIs. Native handlers
  validate the original view/surface and continuation marker placement.

## 8. Deliberate profile limits and follow-ups

- This is bounded legacy Vim script, not Vim9, a plugin host, or the full Vim
  runtime. Runtime/plugin loading, shell/jobs/timers/foreign calls, file-writing
  builtins, interactive function definition and full Vim buffer/tab management
  remain outside scope. Unsupported builtins report errors even when their names
  are recognized. Interactive input/confirmation compatibility functions return
  defaults; dictionary ordering and UTC-only date formatting are approximations.
- Script state, including `g:`, belongs to a shared buffer. Window variables are
  per view; buffer/window/tab introspection is limited and some geometry values
  are placeholders. Native continuation does not transfer into a different core
  or newly selected pane after `:edit`/window switching.
- Native settings are mappings with explicit limits, not complete gVim display
  semantics. See `docs/vimscript.md` for font, wrapping, scrolling, menu and bell
  details. `guicursor` remains recognized but unsupported; backup/undo-file
  settings remain intentionally ignored.
- Syntax-file/color-scheme evaluation retains the separate bounded evaluator in
  `document/syntax/vim/loader/`; merging it requires a determinism/caching review.
- Windows native execution must be validated on Windows. This macOS environment
  has no `dotnet` toolchain; generated bindings and manual integration review do
  not substitute for WinUI diagnostics.
- Editing note: existing core/FFI/header/C# files often use CRLF. Preserve their
  line endings instead of rewriting whole files through Python text-mode I/O.

## 9. Documentation

Updated: AGENTS.md, docs/README.md, docs/macos-development.md, src/win/README.md,
gaps.md, and the new `docs/vimscript.md` compatibility profile. This handoff is
the implementation/validation record; the profile is the user-facing reference.
