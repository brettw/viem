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
