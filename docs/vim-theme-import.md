# Vim theme import

Theme settings' **Import…** turns a Vim color scheme into a Viem theme. The
dialog lists the `.vim` files in the user's Vim runtime and offers **Select
File…**, **Cancel** and **Import**. Choosing a file in Select File… imports it
immediately. The new theme is named after the file stem (`desert.vim` →
`desert`), with ` (2)`, ` (3)`, … added on a case-insensitive collision or for
reserved names such as `default`. It is saved and selected.

| Platform | Directories, in order |
| --- | --- |
| macOS | `~/.vim/colors`, `$XDG_CONFIG_HOME/vim/colors` (default `~/.config/vim/colors`) |
| Windows | `~\vimfiles\colors` (Vim's `$HOME`, defaulting to the user profile), `~\.vim\colors` |

A name found in an earlier directory hides later ones, as `:colorscheme` loads
the first. Hidden files are not listed.

The evaluation and mapping are portable core code
([`colorscheme.rs`](../src/core/document/syntax/vim/colorscheme.rs) and
[`vim_import.rs`](../src/core/document/theme/vim_import.rs)); the native
frontends only list and read files.

## Evaluation

The scheme runs through the Vim syntax loader's bounded setup language
(variables, dictionaries, lists, `if`/`for`/`while`, user functions, `:execute`,
`:call`) as gVim 9.2 with `:syntax on` would load it with `:colorscheme`:

- Vim's default state comes first: 'background' is `light`, and every group has
  Vim's compiled-in or `syncolor.vim` default. `:highlight clear` restores
  those defaults and unsets `g:colors_name`.
- `:highlight` follows Vim: attributes merge into the group, setting attributes
  removes a link, `hi link` without `!` is ignored for a group that already has
  settings, `hi def` applies only to an unset group, `hi clear {group}` restores
  its default link, and `NONE` clears.
- `set background=` (or `let &background`) changes the value later conditions
  see. While unnamed it reloads the defaults Vim reloads; once `g:colors_name` is
  set it does not, matching Vim's recursion guard.
- `has('gui_running')`, `has('gui')`, `has('termguicolors')`, `&termguicolors`
  and Vim 9.2 patch checks are true; `has('nvim')` and platform features are
  false; `&t_Co` is `256`. GUI color names use Vim's tables.
- Statements outside this language (autocommands, nested `:colorscheme`, Lua,
  Vim9 `def` functions, unknown commands or colors) are skipped and reported as
  warnings; the rest of the scheme still applies. Exhausted work budgets reject
  the import. A file with no `:highlight` command is rejected.

An optional test compares the resulting GUI attributes of 58 groups with an
installed Vim: `VIEM_VIM_COLORS=/usr/share/vim/vim91/colors cargo test --lib
installed_vim_resolves`. Vim 9.1's 26 bundled schemes match exactly.

## Base theme

The theme starts from the shipped Paper or Midnight preset embedded in the core,
not the profile's possibly edited copies. Paper is chosen when Normal's `guibg`
is light (relative luminance above 0.179). Without a Normal background, Vim's
final 'background' decides, so a scheme that never sets either is light like
Vim's default state. Schemes that branch on `&background` without setting it
(for example `lunaperche`, `retrobox`) therefore import their light variant.
Fonts, sizes and spacing come from the preset.

## Mapped directly

These keep their Vim meaning.

| Vim | Viem |
| --- | --- |
| `Normal` `guifg`, `guibg` | Theme Text and Canvas colors |
| Standard syntax groups: `Comment`, `Constant`, `String`, `Character`, `Number`, `Boolean`, `Float`, `Identifier`, `Function`, `Statement`, `Conditional`, `Repeat`, `Label`, `Operator`, `Keyword`, `Exception`, `PreProc`, `Include`, `Define`, `Macro`, `PreCondit`, `Type`, `StorageClass`, `Structure`, `Typedef`, `Special`, `SpecialChar`, `Tag`, `Delimiter`, `SpecialComment`, `Debug`, `Error`, `Todo`, `Added`, `Changed`, `Removed`, `DiffAdd`, `DiffChange`, `DiffDelete` | The same-named Code styles. `guifg`, `guibg`, `bold`, `italic`, `underline` and `strikethrough` map to foreground, background, Bold, Italic, Underline and Strikethrough |
| Groups Vim's syntax files link to without a built-in Code style: `Underlined`, `Ignore`, `Title`, `Bold`, `Italic`, `BoldItalic`, `ModeMsg`, `MoreMsg`, `WarningMsg`, `ErrorMsg`, `Question`, `Directory`, `NonText`, `SpecialKey`, `Conceal`, `DiffText`, `LineNr`, `Visual`, `Folded` | New Code styles of the same name, added when they differ from default text. Vim-highlighted Code reaches them through the syntax files' links |
| Neovim capture groups a scheme defines outside `has('nvim')`, such as `@keyword.import` | The Code style for that capture (`Keyword.import`), unless a same-named Vim group exists |
| `Search` | The internal Incremental match style in the Code, Text and Markdown stylesheets |
| `LineNr` | Line number: foreground, gutter background, Bold, Italic |
| `CursorLineNr`, when the scheme defines it | Current line number, relative to Line number |
| `htmlComment` → `Comment` | Markdown Comment |

Every Code style declares only what differs from the style it inherits, so
links reproduce as inheritance (`Conditional` with no declarations follows
`Statement`). Where Viem's default hierarchy differs from Vim's links (Viem
files `Character` under `String`, Vim under `Constant`), the style gets explicit
declarations matching Vim's result.

## Mapped with approximations

| Vim | Viem | Approximation |
| --- | --- | --- |
| `Cursor` | Caret color | The block's color is Cursor's background; its `guifg` (text under the block) is not used because Viem redraws covered text in black or white. Without a background gVim inverts the text, so the caret takes the text color |
| `Visual` | Selection color | Opaque `guibg`. Selection paints beneath unchanged text, so Visual's own `guifg` cannot apply; when the scheme relies on it and Normal text would have contrast below 3:1, the selection is 40% opaque. `gui=reverse` uses the text color |
| `StatusLine` | Status bar in every stylesheet | Colors after `reverse`; its `bold` and `italic` are not applied to status typography. The top border takes the bar's background (flat, like Vim), or its foreground when the bar matches the canvas |
| `StatusLine`, inverted | Status bar command | Vim draws the command line with Normal below the status line; Viem's command field is the status bar's inverse |
| `markdownH1`–`markdownH6` → `htmlH1`–`htmlH6` → `Title` | Markdown Heading 1–6 foreground | Heading typography is kept; Paper's inverted Heading 1 banner background is removed |
| `markdownLinkText` → `htmlLink` → `Underlined` | Markdown Link foreground | Link keeps its underline |
| `markdownCode`, `markdownCodeBlock`, else `String` | Markdown Code and Code Block text | Vim's `markdown.vim` leaves code unhighlighted, so `String` stands in. Code Block's background is a 7% tint of the text color |
| `markdownId` → `Type` | Markdown reference | Vim colors reference identifiers as `Type` |
| `markdownBlockquote` → `Comment` | Block quote border color | Vim colors quoted text, Viem a border |
| Tree-sitter captures without a Vim group | `Keyword.import` ← `Include`; `String.escape`, `String.special`, `Character.special` ← `SpecialChar`; `Function.macro` ← `Macro`; `Number.float` ← `Float`; `Type.definition` ← `Typedef`; `Type.qualifier` ← `StorageClass`; `Attribute` ← `PreProc`; `Text.uri`, `Markup.link` ← `Underlined`; `Markup.heading` ← `Title` | Other captures (`Variable`, `Property`, `Function.builtin`, `Type.builtin`, `Constant.builtin`, `Punctuation.*`, …) inherit from their Viem parents, which carry the Vim colors |
| `undercurl`, `underdouble`, `underdotted`, `underdashed` | Underline | Viem has one underline style |
| `standout` | Reverse video | As gVim draws it |
| `reverse`/`inverse` on any group | Swapped foreground and background | Normal's colors stand in for unset ones |
| `fg`/`bg` color references | Normal's colors | The preset's text or canvas color when Normal has none |
| Preset syntax typography | Replaced | Midnight's slanted casual Comment face and similar per-style fonts are dropped; Vim's `italic` decides |

## Ignored

- **Editor UI without a Viem equivalent:** `CursorLine`, `CursorColumn`,
  `ColorColumn`, `Folded` and `FoldColumn` (no folding), `SignColumn`,
  `CursorLineSign`, `CursorLineFold`, `EndOfBuffer`, `MatchParen`, `VisualNOS`,
  `QuickFixLine`, `LineNrAbove`, `LineNrBelow`, `lCursor`, `CursorIM`,
  `TermCursor`, `Terminal`, `ToolbarLine`, `ToolbarButton`, `Menu`,
  `Scrollbar`, `Tooltip`, `MsgArea`, `MessageWindow`, `PopupNotification`,
  `ComplMatchIns`, `PreInsert`.
- **Native or unsupported features:** `Pmenu`, `PmenuSel`, `PmenuSbar`,
  `PmenuThumb`, `PmenuKind*`, `PmenuExtra*`, `PmenuMatch*`, `PopupSelected` and
  `WildMenu` (native completion and prompts); `TabLine`, `TabLineSel`,
  `TabLineFill`, `TabPanel*` (no tab pages); `SpellBad`, `SpellCap`,
  `SpellLocal`, `SpellRare` (no spellchecking); `StatusLineTerm`,
  `StatusLineTermNC` (no terminal).
- **Covered by another mapping:** `IncSearch` and `CurSearch` (Viem has one
  search style, from `Search`); `StatusLineNC` (status bars do not distinguish
  inactive panes); `VertSplit` (vertical splitters use the status bar's colors).
- **Not theme settings:** `NonText`, `SpecialKey` and `Whitespace` as markers
  (Visible whitespace is an application setting); `ErrorMsg`, `WarningMsg`,
  `ModeMsg`, `MoreMsg`, `Question`, `Directory` and `Title` as messages (they
  still become Code styles for syntax links).
- **Language-specific groups** such as `javaClassDecl` or `vimCommand`. Viem's
  Vim provider resolves them through the syntax file's own links to standard
  groups, so a scheme's overrides would not apply. The Markdown and HTML groups
  above are the exception.
- **Attributes:** `guisp` (undercurl color), `font=`, `nocombine`, `blend`, and
  every terminal attribute (`term`, `cterm`, `ctermfg`, `ctermbg`, `ctermul`).
- **Other settings:** `g:terminal_ansi_colors` and options other than
  'background' (`guifont`, `t_Co`, `termguicolors`, …).
