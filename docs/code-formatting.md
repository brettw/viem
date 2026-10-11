# Code formatting and comments

`gq` and `gw` reflow selected hard lines in Text and Code. `textwidth` defaults
to 80 logical columns; changing it does not enable automatic wrapping. Markdown
Source and WYSIWYG reflow remain unsupported. Fonts and soft wrapping do not
change reflow boundaries or widths.

Code buffers additionally use Vim's `formatoptions`, `comments`, and
`formatlistpat`. These options are shared by every view of a buffer and do not
enter document undo. Text retains its existing prose/list rules; Markdown's
structural editing does not use these options.

The Code default is `formatoptions=qj`. Supported flags are:

| Flag | Behavior |
| --- | --- |
| `q` | Recognize configured comment leaders during `gq`/`gw`, preserving leaders on continuation lines. Without it, comments reflow as ordinary text. |
| `j` | Remove a repeated comment leader when `J` or `:join` joins compatible comment lines. `gJ` and `:join!` remain literal. |
| `r` | Continue comments after Enter in Insert mode. |
| `o` | Continue comments when opening lines with `o`/`O`. |
| `/` | With `o`, suppress continuation of trailing `//` comments after code. Full-line comments still continue. |
| `n` | Recognize list prefixes using `formatlistpat` and give their continuations a hanging indent. Requires `autoindent`. |
| `2` | Use the paragraph's second-line indentation for its continuations. Requires `autoindent`. |
| `w` | Treat trailing whitespace as a paragraph-continuation marker; wrapped continuation lines retain a trailing space. |
| `1` | Keep one-letter words, including a run of them, with the following word when choosing a wrap. |

For example:

```vim
:set fo+=ro
:set fo+=/
:set fo-=j
:set fo?
:set fo&
```

`+=`, `^=`, and `-=` modify values; `?` queries them. `&` and `<` return to
inherited defaults. Unsupported flags and invalid option values fail atomically,
including a `:set` command that also contains otherwise valid changes. The
existing comment-continuation settings and `continuecommentsonenter` /
`continuecommentsonopenline` options control `r` and `o`; an explicit
`formatoptions` override survives application-default changes.

## Comment definitions

The C-family languages previously recognized by reflow receive this default:

```vim
:set comments=:///,://!,://,s1:/*,mb:*,ex:*/
```

Each comma-separated entry has `flags:literal-leader` syntax. Repeated leaders
may use `b` (require whitespace after the leader), `n` (nested leaders), `f`
(first line only), and `O` (do not continue with uppercase `O`). Three-piece
comments use consecutive `s`, `m`, and `e` entries. Their alignment flags and
numeric offsets control generated leaders, and `x` permits the final character
of the end marker to close a freshly generated middle leader. Existing starred
or unstarred block bodies retain their convention. Place longer overlapping
leaders before shorter ones.

A language refresh updates the inherited default, preserving explicit buffer
values. Languages without declared defaults initially have no comment leaders.
An explicit empty value disables comment recognition for reflow, joining, and
line continuation:

```vim
:set comments=
:set comments=b:#
:set comments&
```

Comment editing uses local source context, independently of syntax highlighting.
Block lookup is bounded to 512 preceding hard lines. Nested block comments and
language-runtime indentation are not implemented. Paste and IME commits bypass
per-keystroke continuation assistance.

## List prefixes and limits

`formatlistpat` defaults to Vim's numbered-list pattern,
`^\s*\d\+[\]:.)}\t ]\s*`. To recognize bullet prefixes instead:

```vim
:set fo+=n
:set formatlistpat=^\s*[-+]\s\+
```

Patterns use the existing bounded Vim regular-expression engine, including
magic switches and `\ze`; no Vimscript or external formatter runs. The match
must start at the beginning of the body, consume a nonempty prefix, and leave
body text. A match ending inside a grapheme is rejected. Unsupported pattern
atoms, invalid expressions, and work-budget exhaustion produce an error rather
than silently changing the pattern's meaning. Reflow remains one verified,
undoable transaction and preserves untouched source bytes and encoding.
