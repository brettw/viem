**Vim scripting compatibility audit — November 14, 2026**

Audited commit `5c6b7f0c8603b31fc7a1a7f88e7243bebd6d3a6b` (`Fix register bug.`), including the register fix, editor-facing scripting operations, legacy value semantics, and Vim9 compilation and command handling. The audit found **21 implementation mismatch families**, including a process-aborting stack overflow, a Rust panic, and several operations that silently change text or state differently from Vim. Two additional register compatibility gaps are documented separately below because they are already explicit limitations.

These findings were reproduced through Viem's real `Core` and `:source` path, compared with `/usr/bin/vim` **9.1.1752** with user configuration and viminfo disabled. The repository targets Vim **9.2.0000**; relevant target-version contracts were also consulted in the upstream [builtin documentation](https://github.com/vim/vim/blob/v9.2.0000/runtime/doc/builtin.txt), [expression documentation](https://github.com/vim/vim/blob/v9.2.0000/runtime/doc/eval.txt), and [Vim9 documentation](https://github.com/vim/vim/blob/v9.2.0000/runtime/doc/vim9.txt). This was not an exhaustive comparison with a running Vim 9.2 binary or the entire upstream test suite. No compatibility percentage can be inferred from this audit.

There are 27 reproducer files: 25 safe probes and two isolated crash probes. **All 76 existing focused integration tests passed**, alongside the safe diagnostic driver. Passing existing tests therefore does not cover the mismatches below. No production code or permanent tests were changed. The temporary integration driver was removed after collecting evidence.

Each snippet below is a separate script. Unless specified otherwise, line-oriented probes initialize their own content. P1 denotes crashes or silent content/state changes requiring early attention; P2 denotes other reproducible compatibility failures. The findings are ordered by impact, not by implementation sequence.

**1. [P1] `typename()` aborts the process on cyclic values**

```vim
let L = []
call add(L, L)
echo typename(L)
```

Vim prints `list<list<any>>`. Viem overflows the native stack and aborts with `SIGABRT`. `typename()` recursively visits List and Dictionary contents without cycle detection or a depth budget. The List case was confirmed in an isolated process; the Dictionary branch has the same unbounded recursion in code.

Location: [collections.rs:458](src/core/vimscript/builtins/collections.rs#L458), Dictionary branch at line 467. Evidence: [reproducer](target/vimscript-audit/cases/crash_typename.vim), [crash log](target/vimscript-audit/crash-typename.log).

**2. [P1] Invalid Blob removal ranges panic or mutate instead of failing**

```vim
let B = 0z010203
echo remove(B, 2, 0)
```

Vim reports E979. Viem panics with `slice index starts at 2 but ends at 1`. Separately, `remove(B, 0, 99)` reports E979 and preserves `B` in Vim, but removes the entire Blob in Viem. The implementation validates the starting index but clamps the ending index and passes reversed ranges to `Vec::drain`.

Location: [collections.rs:577](src/core/vimscript/builtins/collections.rs#L577). Evidence: [panic reproducer](target/vimscript-audit/cases/crash_blob_range.vim), [crash log](target/vimscript-audit/crash-blob-range.log), [non-crashing range probe](target/vimscript-audit/cases/legacy_blob_range_validation.vim).

**3. [P1] `getline()` drops literal carriage returns in Unix files**

```vim
set fileformat=unix
call setline(1, "x\r")
echo strlen(getline(1)) char2nr(getline(1)[1])
```

Vim prints `2 13`; Viem prints `1 0`. The source still contains the authored CR, but `getline()` trims every trailing CR and LF, rather than removing only the actual line delimiter. Consequently, the ordinary `setline(1, getline(1))` round trip would delete this content. The scripting search path applies the same trimming.

Locations: [command/scripting.rs:300](src/core/command/scripting.rs#L300), [coordinator/scripting.rs:226](src/core/coordinator/scripting.rs#L226). Evidence: [reproducer](target/vimscript-audit/cases/editor_literal_cr.vim).

**4. [P1] `setline()` changes line structure for embedded LF in a line value**

```vim
call setline(1, ['one', 'two', 'three'])
call setline(1, "a\nb")
echo line('$') strlen(getline(1)) char2nr(getline(1)[1])
```

Vim prints `3 3 10`; Viem prints `4 1 0`. Vim's line API represents an embedded NUL using LF inside the returned or supplied line string. Viem copies the string directly into source, treating that LF as a physical line delimiter. A script processing line values can therefore split one line into two.

Locations: [command/scripting.rs:1110](src/core/command/scripting.rs#L1110), [coordinator/scripting.rs:195](src/core/coordinator/scripting.rs#L195). Evidence: [reproducer](target/vimscript-audit/cases/editor_newline_in_line.vim).

**5. [P1] Vim9 comment stripping corrupts Ex command payloads**

```vim
vim9script
setline(1, 'old')
s/old/ #tag/
echo string(getline(1))
setline(1, 'old')
normal! A #tag
echo string(getline(1))
```

Vim prints `' #tag'` and `'old #tag'`; Viem prints `''` and `'old'`. The generic Vim9 comment scanner strips a whitespace-preceded `#` before dispatching the command, including inside substitute replacement text and normal-mode keystrokes. Comment parsing must respect each command's payload grammar.

Locations: [source.rs:165](src/core/vimscript/source.rs#L165), dispatch at [commands.rs:444](src/core/vimscript/commands.rs#L444). Evidence: [reproducer](target/vimscript-audit/cases/vim9_payloads.vim).

**6. [P1] Invalid Vim9 concatenation is rejected after function side effects**

```vim
vim9script
var effects = 0
def WrongConcat()
  effects += 1
  var text = [] .. 'text'
enddef
try
  WrongConcat()
catch
  echo effects
endtry
```

Vim prints `0`; Viem prints `1`. Compilation assigns concatenation a String result without checking operand types. Viem enters the function and performs preceding effects before reporting the invalid List operand. The probe confirms variable mutation; preceding document edits are a potential consequence of the same missing validation, not a separately executed probe.

Location: [compiler.rs:285](src/core/vimscript/compiler.rs#L285). Evidence: [reproducer](target/vimscript-audit/cases/vim9_concat_validation.vim).

**7. [P2] Writing a named register overwrites the unnamed register**

```vim
call setreg('"', 'keep')
call setreg('a', 'scratch')
echo getreg('"')
let @b = 'temp'
echo getreg('"')
```

Vim prints `keep` twice; Viem prints `scratch`, then `temp`. `write_script()` unconditionally stores the new value in the unnamed slot, contrary to the comment claiming Vim equivalence. This breaks scripts that use a named scratch register while preserving the user's paste contents.

Location: [registers.rs:627](src/core/command/registers.rs#L627). Evidence: [reproducer](target/vimscript-audit/cases/register_unnamed.vim).

**8. [P2] `setreg()` append and explicit type handling are incomplete**

```vim
call setreg('1', 'old', 'c')
call setreg('1', 'new', 'ac')
echo getreg('1')
call setreg('a', 'old', 'l')
call setreg('a', 'new', 'ac')
echo getregtype('a')
```

Vim prints `oldnew` and `v`; Viem prints `new` and `V`. Append is implemented by uppercasing the register name, which cannot request append for numeric registers. The shared append helper also preserves a linewise type when the explicit `c` option should select characterwise output.

Locations: [command/scripting.rs:269](src/core/command/scripting.rs#L269), [registers.rs:674](src/core/command/registers.rs#L674). Evidence: [reproducer](target/vimscript-audit/cases/register_append.vim).

**9. [P2] An empty List creates a register line instead of clearing its contents**

```vim
call setreg('a', [])
echo string(getreg('a')) string(getreg('a', 1, 1)) string(getregtype('a'))
```

Vim prints `'' [] ''`; Viem returns a newline, `['']`, and `V`. With explicit block type, Vim again returns empty contents/list/type, whereas Viem returns `['']` and a block type of width zero. The List conversion always appends a linewise terminator, even when there are no items.

Location: [command/scripting.rs:777](src/core/command/scripting.rs#L777). Evidence: [reproducer](target/vimscript-audit/cases/register_empty.vim).

**10. [P2] Special registers do not preserve the register API contract**

```vim
call setreg('/', ['needle'])
echo string(getreg('/'))
let @= = '1 + 2'
echo string(getreg('=', 1, 1))
echo string(getregtype('/'))
```

Vim accepts the one-item List for `/`, returns `['1 + 2']` for the expression register's requested List representation, and reports search-register type `v`. Viem rejects the List with E730, returns the expression as a String, and reports an empty search-register type. The special write branches call String conversion directly, and the expression read returns before handling the List flag.

Locations: [command/scripting.rs:260](src/core/command/scripting.rs#L260), [line 475](src/core/command/scripting.rs#L475), [line 500](src/core/command/scripting.rs#L500). Evidence: [reproducer](target/vimscript-audit/cases/register_special.vim).

**11. [P2] Cursor APIs conflate byte, character, virtual, and encoded-source columns**

```vim
set tabstop=8
call setline(1, "é\tx")
call cursor(1, 4)
echo col('.') charcol('.') virtcol('.') getcharpos('.')[2]
call setcursorcharpos(1, 3)
echo col('.')
```

Vim prints `4 3 9 3`, then `4`; Viem prints `4 4 4 4`, then `3`. `setcharpos()` has the same setter defect. An additional Latin-1 probe with `éxy` and `cursor(1, 3)` prints `3 2 éxy` in Vim but `3 3 éxy` in Viem: the cursor bridge uses encoded source offsets where Vim's internal UTF-8 byte column is required. Scripts navigating Unicode text or tabs can target the wrong character.

Locations: [command/scripting.rs:285](src/core/command/scripting.rs#L285), [line 354](src/core/command/scripting.rs#L354), [line 1125](src/core/command/scripting.rs#L1125), [coordinator/scripting.rs:344](src/core/coordinator/scripting.rs#L344). Evidence: [column probe](target/vimscript-audit/cases/editor_columns.vim), [encoding probe](target/vimscript-audit/cases/editor_encoded_columns.vim).

**12. [P2] `append()` moves the cursor onto different content**

```vim
call setline(1, ['one', 'two', 'three'])
call cursor(2, 2)
call append(0, ['added'])
echo line('.') col('.') getline('.')
```

Vim prints `3 2 two`; Viem prints `2 2 one`. The bridge restores the old numeric line and column after insertion rather than retaining the cursor's content position. Subsequent script edits can operate on a different line.

Location: [command/scripting.rs:1158](src/core/command/scripting.rs#L1158). Evidence: [reproducer](target/vimscript-audit/cases/editor_append_cursor.vim).

**13. [P2] `search()` ignores `nowrapscan` and its skip expression**

With lines `one`, `two`, `three`, cursor on line 3, and `set nowrapscan`, `search('one')` returns `0` and leaves the cursor on line 3 in Vim. Viem wraps, returns `1`, and moves to line 1. With lines `one`, `two`, `one` and cursor on line 1, `search('one', 'W', 0, 0, '1')` returns `0` in Vim because every match is skipped; Viem returns `3` and moves there. The implementation sets wrapping solely from the absence of `W` and never evaluates the fifth argument.

Location: [coordinator/scripting.rs:213](src/core/coordinator/scripting.rs#L213). Evidence: [nowrapscan probe](target/vimscript-audit/cases/editor_search_nowrap.vim), [skip probe](target/vimscript-audit/cases/editor_search_skip.vim).

**14. [P2] Value locks are ignored or enforced at the wrong level**

Four confirmed cases expose inconsistent lock handling:

| Operation | Vim | Viem |
| --- | --- | --- |
| `let L=[1,2]`, `lockvar L[0]`, then `let L[0]=8` | E741, unchanged | `[8,2]` |
| `let L=[1,2]`, `lockvar 1 L`, then `let L[0]=8` | `[8,2]` | E741, unchanged |
| `lockvar D`, then `map(D, 'v:val + 1')` for `D={'a':1}` | E741, unchanged | `{'a':2}` |
| `lockvar B`, then `remove(B,0)` for `B=0z0102` | E741, unchanged | `0z02` |

Member locking, binding/container locking, and mutation authorization need a consistent model. Fixing only `:let` would leave builtin mutations inconsistent.

Locations: [let_cmd.rs:857](src/core/vimscript/let_cmd.rs#L857), [line 242](src/core/vimscript/let_cmd.rs#L242), [collections.rs:713](src/core/vimscript/builtins/collections.rs#L713), [line 561](src/core/vimscript/builtins/collections.rs#L561). Evidence: [member lock](target/vimscript-audit/cases/legacy_member_lock.vim), [shallow lock](target/vimscript-audit/cases/legacy_shallow_lock.vim), [builtin mutators](target/vimscript-audit/cases/legacy_mutators_locked_values.vim).

**15. [P2] `map()` callbacks cannot observe earlier replacements**

```vim
let L = [1, 2, 3]
call map(L, 'v:val + L[0]')
echo L
```

Vim prints `[2, 4, 5]`; Viem prints `[2, 3, 4]`. Viem evaluates callbacks against a snapshot and delays publishing replacements until the callbacks finish. Vim's later callbacks observe earlier changes to the same List.

Location: [collections.rs:674](src/core/vimscript/builtins/collections.rs#L674), publication at line 708. Evidence: [reproducer](target/vimscript-audit/cases/legacy_map_visibility.vim).

**16. [P2] Legacy `for` skips items after removal from the iterated List**

```vim
let L = [1, 2, 3]
for item in L
  call remove(L, 0)
endfor
echo L
```

Vim prints `[]`; Viem prints `[3]`. Legacy Vim tracks the next List item during iteration. Viem uses an index-based loop, applying Vim9-style traversal to legacy scripts as well. Changing this globally would risk breaking the intentionally different Vim9 behavior.

Location: [commands.rs:1388](src/core/vimscript/commands.rs#L1388). Evidence: [reproducer](target/vimscript-audit/cases/legacy_for_removal.vim).

**17. [P2] `deepcopy(..., 1)` preserves aliases that must become independent**

```vim
let child = [1]
let L = deepcopy([child, child], 1)
let L[0][0] = 8
echo L
```

Vim prints `[[8], [1]]`; Viem prints `[[8], [8]]`. The optional `noref` argument is accepted but ignored. Code requesting independent copies still gets shared nested values.

Location: [collections.rs:383](src/core/vimscript/builtins/collections.rs#L383). Evidence: [reproducer](target/vimscript-audit/cases/legacy_deepcopy_noref.vim).

**18. [P2] Out-of-range List assignment appends instead of failing**

```vim
let L = [1]
let L[1] = 2
echo L
```

Vim reports E684 and leaves `[1]`; Viem silently creates `[1, 2]`. The implementation explicitly appends when the assigned index equals the List length. This changes both error handling and state after an invalid assignment.

Location: [let_cmd.rs:250](src/core/vimscript/let_cmd.rs#L250). Evidence: [reproducer](target/vimscript-audit/cases/legacy_list_index_bounds.vim).

**19. [P2] Vim9 compilation parses valid Ex commands as expressions**

```vim
vim9script
def Replace()
  s/old/(new)/
enddef
setline(1, 'old')
Replace()
```

Vim changes the line to `(new)`; Viem rejects compilation with E121 for undefined variable `s`. The fallback treats almost any command containing `(` as an expression. `legacy echo len([1])` is likewise rejected for undefined variable `legacy`; both are valid commands within a Vim9 function. Command grammar must be resolved before deciding how arguments are analyzed.

Location: [compiler.rs:961](src/core/vimscript/compiler.rs#L961). Evidence: [substitute probe](target/vimscript-audit/cases/vim9_ex_compile.vim), [cross-dialect probe](target/vimscript-audit/cases/vim9_initial.vim). The latter also contains a compile-only `set foldexpr=...` case; the substitute and `legacy` cases establish this defect without depending on support for that option.

**20. [P2] Constant-false `elseif` branches are incorrectly validated**

```vim
vim9script
def Skipped(): number
  if false
    NoSuchCommand
  elseif false
    NoSuchCommand
  endif
  return 7
enddef
echo Skipped()
```

Vim prints `7`; Viem rejects the unreachable command with E492. Constant evaluation of `elseif` is gated by the activity of the preceding arm, so a preceding false arm prevents recognizing that the next arm is also false. This breaks conditional compatibility guards.

Location: [compiler.rs:480](src/core/vimscript/compiler.rs#L480). Evidence: [reproducer](target/vimscript-audit/cases/vim9_initial.vim).

**21. [P2] Vim9 typed `for` bindings are rejected**

```vim
vim9script
def Sum(): number
  var total = 0
  for item: number in [1, 2]
    total += item
  endfor
  return total
enddef
echo Sum()
```

Vim prints `3`; Viem reports E1017. The compiler validates `item: number` as a single variable name instead of parsing its type annotation. The runtime loop parser also lacks annotation handling, so compilation and execution need to agree on the corrected binding syntax.

Locations: [compiler.rs:846](src/core/vimscript/compiler.rs#L846), [commands.rs:1243](src/core/vimscript/commands.rs#L1243). Evidence: [reproducer](target/vimscript-audit/cases/vim9_initial.vim).

**Additional confirmed gaps already covered by documented limitations**

- **Explicit block-register widths are discarded.** `setreg('a', ['x', 'yy'], 'b5')` gives CTRL-V followed by `5` from Vim's `getregtype()`, but CTRL-V followed by `2` in Viem. The new [register documentation](docs/vim-registers-and-marks.md) explicitly says supplied widths are ignored. This remains incompatible with saving/restoring block registers exactly. Location: [command/scripting.rs:763](src/core/command/scripting.rs#L763). [Probe](target/vimscript-audit/cases/register_block.vim).
- **`getreginfo()` and the corresponding Dictionary restoration path are missing.** `exists('*getreginfo')` is `1` in Vim and `0` in Viem; calling it reports an unsupported-function error. The current host profile already acknowledges incomplete builtin coverage, but this API is directly relevant to reliable register preservation. The runtime's value conversion also does not implement the Dictionary form accepted by `setreg()`. [Probe](target/vimscript-audit/cases/register_info.vim).

**Implications for the 100% compatibility goal**

The existing [host profile](docs/vimscript.md) intentionally differs from Vim in process-global versus buffer-local script state, buffer/window/tab introspection, mapping scope, plugin loading, external effects, interactive functions, dictionary iteration order, and some return-value stubs. Fixing the implementation bugs above will not remove those differences. Literal 100% scripting compatibility requires revisiting that profile; otherwise the claim must be limited to a precisely defined supported profile.

A useful sequence is to fix the crash and text-changing cases first, complete register round-trip behavior, then address the remaining language and editor API mismatches. Convert each reproducer into an ordinary regression test asserting Vim's result, including unchanged state on rejected operations. Add command-entry and source/undo coverage for fixes that alter editor mutations.

For sustained compatibility work, pin an executable Vim 9.2 oracle and expand differential testing beyond the current hand-selected fixtures. Track builtin and Ex-command support with argument-level semantics: accepted optional arguments must not silently do nothing. Exercise upstream legacy and Vim9 language tests where the host permits, and record host exclusions explicitly rather than counting them as passes. This audit does not establish coverage of unexamined commands, error diagnostics, all object/import behavior, or native integrations.

**Evidence and reproduction**

- [All scripts and paired outputs, as JSON](target/vimscript-audit/comparisons.json). Each script also has `.out` (Vim) and `.viem.out` (Viem) siblings under `target/vimscript-audit/cases/`; crash cases use separate logs.
- [Preserved diagnostic driver](target/vimscript-audit/probe_driver.rs). It creates a fresh PlainText Core containing `one`, `two`, `three` for each script, submits `:source`, and records messages, status, and final source. Its source display uses UTF-8 lossy formatting, so the Latin-1 probe's display must not be mistaken for raw-byte corruption.
- [Focused run log](target/vimscript-audit/focused-existing-and-probes.log): `python3 scripts/validate.py focused --tests all --filter audit_vimscript_tmp:: --filter vimscript_ --filter registers_marks:: --filter special_registers::`; 77 passed, consisting of 76 existing tests and one diagnostic capture test. The diagnostic test collects observations; it does not assert conformance.
- Isolated crash runs used `VIEM_AUDIT_CASE=crash_blob_range` and `VIEM_AUDIT_CASE=crash_typename` with `python3 scripts/validate.py focused --tests all --filter audit_vimscript_tmp::`. Core dumps were disabled. The driver skips crash-prefixed cases unless selected explicitly.
- Original validation manifests are under `target/validation/20261114-111232-359638/`, `target/validation/20261114-111315-552082/`, and `target/validation/20261114-111347-278983/`.

The temporary test registration and test file have been removed. To rerun the driver, copy the preserved driver to `tests/all/audit_vimscript_tmp.rs` and register `mod audit_vimscript_tmp;` in `tests/all/main.rs`, then use the validator commands above. `scripts/vimscript-oracle.sh` can regenerate the Vim recordings. The linked `target/` evidence is local and ignored by Git; this report contains the findings and minimal examples so it remains useful after build-output cleanup.
