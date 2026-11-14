# Vim script compatibility audit

Differential audit of Viem's Vim script support against real Vim, run on
2026-11-14 at commit `5c6b7f0`. About 1,750 probes ran through both
`/usr/bin/vim` (9.1.1752, the repository's recorded oracle) and Viem; roughly
150 behaved differently. The pure language engine matched on 937 of 1,077
probes. The editor-facing builtins matched far less often.

Nothing listed here has been fixed. Each item shows Vim's result where it
differs.

## Method

- **Engine probes** ran through the in-memory host with `VIEM_DEBUG_SCRIPT`
  (see [Vim script validation](docs/vimscript.md#validation)).
- **Editor probes** ran through a temporary integration test that sourced a
  script into a real `Core` with a six-line buffer, and through separately
  typed command lines for the items marked "confirmed across command lines".
  That test is not in the tree.
- Each probe was wrapped in `try`/`catch` and printed `string(expr)`, so both
  sides report values and exception text in the same form.

Some error numbers differ only because the oracle is 9.1 and the target is
9.2 (E896 against E1531, for example). Those are not listed.

## Serious: wrong edits or lost control flow

1. **`setreg()` is invisible to puts in the same run.**
   `let @a = 'L1' | call setreg('a', 'word') | put a` inserts `L1`.
   `getreg('a')` returns `word` in between, and `"ap` behaves the same way. A
   put on a later command line sees the new value, and `let @a = …` is not
   affected. Confirmed across command lines.
2. **`:normal` ignores the current column.** `:call cursor(1, 5)` then
   `:normal! w` leaves the cursor at column 5 (Vim: 9). Two separate
   `:normal! w` lines also end at 5, while `:normal! ww` reaches 9. Puts and
   inserts after an earlier `:normal` therefore land in the wrong place.
   Confirmed across command lines.
3. **Unsupported Normal commands abort the whole script and bypass `try`.**
   `try | normal! "a]p | catch | … | endtry | echo 'after'` runs neither the
   catch nor the echo. The same happens for `normal! qaq` and
   `i<C-R><C-R>a`. A sourced file stops at the first one.
4. **`setline()` and `append()` drop empty lines when the buffer has no final
   newline.** `%d _ | call setline(1, ['a', '', 'b'])` yields `['a', 'b']`.
   `append(1, ['', 'b', '', ''])` on a one-line unterminated buffer loses the
   last empty line.

## Registers and marks

- **Appending:** Vim keeps the type of the appended text. Appending
  characterwise `y` to linewise `x` gives `x\ny` typed `v`; Viem gives
  `x\ny\n` typed `V`. Appending linewise to characterwise gives `xy\n`; Viem
  inserts a break. This applies to `let @A`, `setreg('A', …)` and the `a`
  option.
- **Unnamed register:** `let @a` and `setreg('a', …)` overwrite `@"` in Viem.
  Vim leaves `@"` alone. The comment on `setreg_options` in
  `src/core/command/scripting.rs` says every write updates it, which is the
  mismatch.
- **Value forms:**
  - `let @a = ['x', 'y']` should be E730.
  - `setreg('a', [])` should give an empty characterwise register; Viem gives
    a linewise `\n`.
  - `setreg('/', ['pat'])` fails with E730.
  - `setreg()` rejects the `getreginfo()` dictionary form, and `getreginfo()`
    is missing.
  - `setreg('!', 'x')` succeeds instead of E354.
  - `setreg('a', 'x', 'q')` reports E475; Vim returns 0.
- **`@=`** returns the evaluated result; Vim returns the expression text.
- **Block width:** Vim stores an explicit width (`'b5'` gives `^V5`) and
  measures in display cells (`日本` is 4). Viem reports the widest row in
  characters. [Registers and marks](docs/vim-registers-and-marks.md) records
  ignoring the width as deliberate; the cell count is still a difference.
- **Marks:**
  - `setpos()` on `'a`, `'A`, `'<` and `'>` does nothing and returns -1.
  - `:mark b` is parsed as `:marks`, `2mark d` is rejected, and `:k` is
    missing.
  - `''` and `'A` lose their column.
  - Linewise `'>` and `']` should report column 2147483647.
  - `getpos('x')` returns `[0, 0, 1, 0]` (Vim: all zeros).

## Position and buffer builtins

- `col([l, c])`, `virtcol([l, c])` and `charcol([l, c])` return 0,
  `line([2, 1])` errors, and `line(2)` returns 2 instead of 0. `col(1)` should
  be E1222.
- `virtcol()` and `curswant` ignore tab expansion, and the `curswant` argument
  of `cursor()` and `setpos()` is dropped.
- `cursor(-1, -1)` returns -1 instead of E475, and `setpos('.', [1, 1])`
  returns 0 instead of -1.
- `search()`:
  - ignores `nowrapscan` and `smartcase`;
  - returns the wrong sub-pattern number for `'p'`, and `searchpos()` omits
    it;
  - accepts invalid flags, ignores the skip expression, and treats an empty
    pattern as a match (Vim: E35);
  - cannot match across lines with `\n`.
- Queries for a nonexistent buffer or window answer for the current one:
  `getbufline(99, 1)`, `getbufoneline(99, 1)`, `bufexists(99)`,
  `bufnr('nosuch')`, `getbufvar(99, …)`, `getcurpos(99)`, `winwidth(99)`.
- `setline(1, "a\nb")` and `append(1, "a\nb")` make two lines; Vim makes one.
- `setline(1, {})` raises E731; Vim returns 0.
- `append(0, …)` does not shift the cursor line.
- `feedkeys(…, 'x')` does not execute.
- `setbufvar()` and `setwinvar()` reject `&option`, and
  `setwinvar(0, 'wv', 7)` does not create `w:wv`.
- `wordcount()` lacks the `cursor_*` keys, and `byte2line(9999)` returns a
  line.
- `histnr('nosuch')` returns 0 (Vim: -1), `histadd('nosuch', 'x')` returns 1
  (Vim: 0), and `histnr('/')` does not count an added entry.
- `expand('nosuch*glob')` returns an empty string (Vim: the pattern), and
  `expand('<slnum>')` is empty inside a sourced script.
- `echo 'a' search('two') 'b'` arrives as two messages.
- A two-key mapping `zq` is not applied by `:normal zq`, though single-key `Q`
  is. Cause not investigated.
- `nrformats` defaults to `bin,hex`, so Ctrl-A on `007` gives `008` (Vim
  without `defaults.vim`: `010`), and `set nrformats=…` is refused.
- Missing:
  - `deletebufline` (which [Vim script support](docs/vimscript.md) lists as
    supported), `appendbufline`, `setbufline`;
  - `getcursorcharpos`, `searchpair`, `searchpairpos`, `searchcount`,
    `searchdecl`;
  - `getmarklist`, `getjumplist`, `undotree`;
  - `getbufinfo`, `getwininfo`, `gettabinfo`, `win_id2win`, `bufadd`;
  - `:@q`, `:*@q` and `:undojoin`.

## Language engine

- **Regex:**
  - `.` does not match a newline in a string, while `^` and `$` match at
    embedded newlines.
  - `match('hello', '^l', 2)` returns -1 (Vim: 2).
  - `[[:alpha:]]` is ASCII-only.
  - `[[=e=]]`, `\%C`, `\Z`, `\%V` and `\%#` are rejected.
  - `a\{2,1}` is rejected (Vim accepts the reversed range), and `a**` is
    accepted (Vim: E61).
  - A base character does not match together with its combining mark:
    `matchstr("é", '.')` returns `e`.
  - Every pattern error is E383 rather than E54, E55, E64 and so on, which
    breaks `catch /E54/`.
- **`split()`:** `split('abc', '')` splits per character (Vim: `['abc']`), and
  the default separator does not split on newline.
- **`sort()`:** mixed types order numbers before strings (Vim: strings first),
  `'l'` is not locale order, and `'N'` accepts a Float (Vim: E805).
- **JSON:**
  - `json_decode()` rejects trailing commas, `NaN`, `Infinity` and `"\x"`,
    accepts duplicate keys (Vim: E938), and decodes `-0` as a Float.
  - `js_decode()` rejects unquoted keys, single quotes and elisions.
  - `json_encode()` of a Funcref reports E474 (Vim: E1161).
- **`printf()`:**
  - `%s` width and precision count characters where Vim counts bytes, and `%S`
    width counts characters where Vim counts cells.
  - `%1$*2$d`, `%5%`, `%03s` and a trailing `%` differ.
  - `%s` of a Funcref prints `function('len')` (Vim: `len`).
  - `%c` with 0, a String or a value above 255 prints a space (Vim: nothing).
  - `%.3g` and `%.10g` drop trailing zeros that Vim keeps.
- **Smaller ones:**
  - `toupper('ß')` gives `SS`.
  - `abs('-3')` errors.
  - `[1] == [1.0]` is true.
  - `v:none == v:null` is true.
  - `{1.5: 'x'}` errors.
  - `funcref('toupper')` succeeds (Vim: E700).
  - `get(0z0102, 5)` is 0 (Vim: -1).
  - `get(function('len'), 'arity')` reports no required arguments.
  - `typename()` of a Funcref is just `func`.
  - `exists(':2match')` is 0 (Vim: 2).
  - `"\<S-Tab>"` has length 5 (Vim: 3).
  - `nr2char(-1)`, `utf16idx()` past the end, `charclass('日')` and
    `iconv(…, 'latin1')` differ.
  - `glob2regpat('*.txt')`, `fnameescape('-')`, `shellescape("a\nb", 1)`,
    `str2float('0x10')`, `strwidth("a\tb")`, `submatch(0, 1)` and `matchstr()`
    on a list differ.
  - `reverse(1)` returns 0 instead of erroring.
  - `:echo` prints control characters raw; Vim shows `^A`, `^V` and so on.

## Not covered

- About 115 probes in the last editor set never ran in Viem because the script
  aborted at an unsupported `:normal`: `fnamemodify`, `simplify`, file and
  glob functions, `strftime`, `getcompletion`, and the remaining window and
  syntax queries.
- Vim9 script, autocommands, user commands, and options beyond the few used in
  probes were not probed.
- Some differences may be deliberate under the host profile. Only the block
  register width and the `nrformats` default were checked against the docs.
