# Registers and marks

Viem resolves special registers through the same command paths as named
registers: Normal and Visual put, Insert/Replace Ctrl-R, prompt Ctrl-R,
`:put`, and script reads such as `@%` and `getreg('%')`.

| Register | Contents |
| --- | --- |
| `/` | Last search pattern; scripts can assign it with `:let` or `setreg()` |
| `:` | Last submitted Ex command |
| `%` | Current filename, including the native buffer filename before a persistence binding exists |
| `#` | The current view's alternate filename, also used by Control-^ |
| `~` | Last accepted native text drop in this buffer |
| `=` | A Vim script expression |

Filename registers preserve the supplied path. A path that cannot be represented
as Unicode fails explicitly. A missing current or alternate filename produces an
empty script value or an actionable register error, as appropriate to the read.
Yank and delete cannot write these special registers. Ordinary clipboard
operations do not overwrite the drop register, and a rejected drop preserves it.
Text drops use the native Paste transaction, including its undo behavior.

Selecting `=` opens an expression prompt with its own history. Enter accepts it;
an empty prompt reuses the previous expression. Escape cancels it. Inside another
prompt, Ctrl-R `=` preserves the outer text, selection and caret until the result
is inserted. Expressions use the shared Vim script evaluator with text and editor
mutations locked. They can read script variables, options, registers and editor
queries and call script functions. Invalid expressions and invalid Unicode
results do not edit the document. Numbers become text; lists become lines.
`:put =expression` and `:let @= = 'expression'` use the same evaluator.
`getreg('=', 1)` returns the stored expression instead of evaluating it.

| Mark | Location |
| --- | --- |
| `[` / `]` | First/last affected grapheme of the latest change or yank |
| `.` | Latest change position |
| `^` | Position where Insert or Replace ended, before stepping back into Normal mode |
| `'` / backtick | Position before the latest jump; repeated jumps exchange the two positions |
| `A`–`Z` | Application-wide file marks set with `mA` through `mZ` |

Apostrophe jumps select the first nonblank grapheme of the marked line;
backtick jumps keep the exact marked position. Local mark motions also work
with operators and Ex addresses. A cross-file mark cannot define an operator
range in a different buffer.

Buffer marks follow edits and are shared by that buffer's views; the previous
jump position stays view-local. Uppercase mark ownership is shared across
windows: assigning a letter in another buffer replaces its owner. A jump focuses
an existing view when available; otherwise it opens the marked file using the
usual modified-buffer checks. Open marks use rebased positions. Closed marks
retain a physical file location, resolved against the reopened file; an invalid
location fails instead of applying an old snapshot offset. The registry retains
at most 26 locations and paths, without keeping closed documents alive. These
marks last for the application session and are not persisted across restarts.
