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

## Register scope

As in Vim, one application-wide bank holds the unnamed register `"`, the
numbered registers `0`–`9`, the named registers `a`–`z`, the small-delete
register `-`, the last-insert register `.` and recorded macros. A yank in one
buffer can be put in any other, and the bank outlives the buffer that wrote
it. `+` and `*` are the system clipboard. The bank lasts for the application
session; unlike Vim, registers are not saved across restarts, and the
`viminfo` option does not change that.

Each buffer keeps a copy of the bank. A command turn starts from the newest
bank and, when it ends, publishes only the registers it wrote. If commands in
two buffers write registers at the same time, each register keeps the write
from the command that finished last, so the numbered registers may interleave
both deletions.

`%` belongs to the buffer and `#` to the view, as in Vim.

### Per-buffer differences from Vim

Vim keeps the following state for the whole session. Viem keeps it per buffer,
shared by that buffer's views, so it is unavailable in other buffers:

| State | Viem behavior |
| --- | --- |
| `:` register | The last Ex command submitted in this buffer |
| `/` register | The last search pattern in this buffer, also used by `n`, `N`, `g&` and `hlsearch` |
| `=` register | The last expression evaluated in this buffer |
| `~` register | The last text dropped into this buffer |
| Search, Ex and expression histories | Prompt history of this buffer |
| `.` repeat | Repeats the last change made in this buffer |
| `@@` | Repeats the last register executed in this buffer |
| Macro recording | `q` records only input to the buffer where it started |
| `:nohlsearch` | Suppresses search highlighting in this buffer only |
| Previous substitute | `&`, `g&` and `:&` repeat the last `:s` in this buffer |

Mappings, abbreviations and script state are also per buffer; see
[Vim script](vimscript.md) for their scope and for option scope.

## Marks

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
