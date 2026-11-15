**Vim scripting compatibility audit — November 14, 2026**

Audited commit `5c6b7f0c8603b31fc7a1a7f88e7243bebd6d3a6b` (`Fix register bug.`), including the register fix, editor-facing scripting operations, legacy value semantics, and Vim9 compilation and command handling. The audit found **21 implementation mismatch families**, including a process-aborting stack overflow, a Rust panic, and several operations that silently change text or state differently from Vim. Two additional register compatibility gaps are documented separately below because they are already explicit limitations.

These findings were reproduced through Viem's real `Core` and `:source` path, compared with `/usr/bin/vim` **9.1.1752** with user configuration and viminfo disabled. The repository targets Vim **9.2.0000**; relevant target-version contracts were also consulted in the upstream [builtin documentation](https://github.com/vim/vim/blob/v9.2.0000/runtime/doc/builtin.txt), [expression documentation](https://github.com/vim/vim/blob/v9.2.0000/runtime/doc/eval.txt), and [Vim9 documentation](https://github.com/vim/vim/blob/v9.2.0000/runtime/doc/vim9.txt). This was not an exhaustive comparison with a running Vim 9.2 binary or the entire upstream test suite. No compatibility percentage can be inferred from this audit.

There are 27 reproducer files: 25 safe probes and two isolated crash probes. **All 76 existing focused integration tests passed**, alongside the safe diagnostic driver. Passing existing tests therefore does not cover the mismatches below. No production code or permanent tests were changed. The temporary integration driver was removed after collecting evidence.

Each snippet below is a separate script. Unless specified otherwise, line-oriented probes initialize their own content. P1 denotes crashes or silent content/state changes requiring early attention; P2 denotes other reproducible compatibility failures. The findings are ordered by impact, not by implementation sequence.

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

**22. [P1] Line values do not swap CR and LF in Mac-format files**

```vim
" Buffer read with fileformat=mac from the bytes  a LF b CR next CR
echo line('$') strlen(getline(1)) char2nr(getline(1)[1])
call setline(1, getline(1))
call setline(2, "x\r")
echo line('$') strlen(getline(2)) char2nr(getline(2)[1])
```

Vim prints `2 3 13`, then `2 2 13`, and writes `a LF b CR x LF CR`. In a Mac-format buffer Vim reads a literal LF inside a line as CR, and writes a CR from a line value back as LF, because CR is the line delimiter. Viem prints `2 3 10`. Its round trip then replaces the literal LF with NUL, because `setline()` reads every LF in a line value as Vim's spelling of NUL. That replacement was introduced by the fix for embedded LF in line values. `setline(2, "x\r")` changes nothing and reports no error, where Vim stores a one-line `x<CR>`.

Line values need the file-format mapping both ways: in Mac format, source LF becomes CR in a value, and CR in a value becomes source LF; LF in a value still means NUL in every format.

Locations: `script_line_value()` and `script_line_source()` in [command/scripting.rs](src/core/command/scripting.rs), used by `script_line_text()` and by `lines_argument()` in [coordinator/scripting.rs](src/core/coordinator/scripting.rs). Evidence: Vim was probed with `scripts/vimscript-oracle.sh`; Viem with a temporary `Core` test using `FileFormat::Mac`, since removed.

**23. [P2] Vim9 concatenation splits a Float literal on its right side**

```vim
vim9script
echo 'a' .. 2.5
echo 'a' .. 2.5e1
```

Vim prints `a2.5` and `a25.0`; Viem prints `a2`, then reports `E15: Invalid expression: ".5"` and stops the script. Legacy script deliberately reads a number after `.` or `..` as an integer, so `'a' . 2.5` is `a25` in both editors. Vim9 drops that rule, but Viem's expression parser disables Float literals on the right of every concatenation regardless of dialect. `1 .. 2.5` fails the same way, while a Float held in a variable (`'a' .. n`) concatenates correctly.

Location: the `want_string` check before Float literal parsing in [expr.rs](src/core/vimscript/expr.rs), reached from concatenation in `eval6`. Evidence: both editors were probed with the script above.

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
