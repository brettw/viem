# Markdown tables

This is the product specification for table support on macOS and Windows.
Current support and remaining gaps are tracked in
[MARKDOWN_GAPS.md](../MARKDOWN_GAPS.md). The source-preservation,
transaction, input, style, and performance requirements in
[AGENTS.md](../AGENTS.md) apply throughout.

The target is GitHub Flavored Markdown (GFM) pipe tables, with Word-like editing
where Markdown can represent the result. The picker dimensions, counting body
rows separately from GFM's mandatory header, content sizing, and cell-break
behavior below are deliberate Viem choices.
Microsoft's documentation informs the interaction model; it does not override
these requirements.

## GitHub compatibility and representation

Accept the full [GFM table grammar](https://github.github.com/gfm/#tables-extension-),
including:

- One header row, a delimiter row, and zero or more body rows. Header and
  delimiter cell counts must agree; otherwise the construct is ordinary Markdown.
- Optional outer pipes, varying between rows; trimmed cell-edge whitespace;
  empty cells; short body rows padded semantically with empty cells; and excess
  body cells ignored by rendering.
- Delimiter cells containing hyphens and optional alignment colons. Accept
  one-hyphen forms as well as longer runs. Leading, trailing, or both colons
  specify left, right, or center alignment respectively.
- Inline Markdown within cells, including escaped pipes inside code spans.
  An unescaped pipe separates cells even within backticks; entities are not
  structural pipe separators. Blocks cannot be nested inside cells.
- GFM block precedence and termination, including tables within list/quote
  owners, blank lines, interrupting blocks, and continuation rows without pipes.
  Do not recognize tables inside code or passive HTML blocks.

New tables use outer pipes, readable cell padding, at least three delimiter
hyphens, and blank separation where required. This follows
[GitHub's authoring guidance](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/organizing-information-with-tables)
without rejecting shorter valid imported spellings. Preserve unspecified column
alignment separately from explicit left alignment. Viem displays unspecified
alignment as left-aligned. Header and body cells use the same column alignment.

Inline interpretation is shared with Markdown outside tables. Preserve Viem's
documented image/reference/comment presentation and passive-HTML restrictions;
table support does not load images or enable active content. Existing inline
compatibility gaps remain explicit in the gap inventory.

GFM has no merged cells, row/column spans, nested tables, multiple header rows,
block paragraphs/lists/code fences inside cells, or authored column widths.
Do not offer those Word features or serialize them through hidden metadata or
authored CSS. Arbitrary HTML `<table>` editing, including spans, is a separate
compatibility project; this specification does not promote literal HTML tables
into editable pipe tables.

## Insert Table control

Add an **Insert Table** toolbar button to Markdown, available in both Source and
WYSIWYG. Its icon is a grid two rectangles across and three high, with each
rectangle twice as wide as high. All strokes have the same thin weight. Give the
button an accessible name and tooltip. Text and Code retain their existing
toolbar policy.

The picker opens below and to the right of the button when space permits. It
starts with ten columns and ten body rows. Its label shows the active dimensions,
for example **3 × 4 Table**, with an explicit **3 columns, 4 body rows + header**
description so that the count cannot be mistaken for total rows. Every accepted
size has at least one column and one body row. A 3 × 4 choice inserts five visible
rows, plus the source-only delimiter row.

### Pointer interaction

| Gesture | Required behavior |
| --- | --- |
| Press and release the toolbar button without dragging | Open the picker and leave it open; that release does not insert a table. |
| Move over an open picker | Highlight the rectangle from its upper-left cell through the hovered cell and update dimensions. |
| Press and release a picker cell | Insert the highlighted size on release and close the picker. |
| Press a picker cell and drag | Capture the pointer and track the same anchored rectangle until release. |
| Press the toolbar button and drag | Open the picker and enter that same captured drag interaction after the native drag threshold. |
| Click outside an open picker, press Escape, lose activation, or lose pointer capture | Cancel and close without insertion. |

While dragging, grow the grid rightward/downward as needed to put a cell under
the pointer, up to **20 columns × 50 body rows**. Growth must not move the grid's
origin or change the meaning of an already highlighted cell. Retain the grown
extent during that opening; selection may shrink when the pointer moves back.
Hover without a pressed button previews the existing grid but does not grow it.

Release above or left of the grid cancels, even when the other coordinate lies
beyond the right or bottom edge. Otherwise accept the cell reached after growth
and scrolling. Beyond the 20-column or 50-row logical limit, clamp only the
exceeded dimension and accept the other indicated dimension. Popup chrome is
not a cell. Cancelling releases capture, clears highlighting, and restores
focus without changing document selection, mode, dirty state, or history.

Fit the popup to the usable screen, choosing another side before interaction
when necessary. A grid too large for the available screen gets a scrollable
viewport and bounded edge autoscroll during captured dragging. Keep the active
logical cell visible, retain its coordinates across scrolling, and make all
20 × 50 choices reachable. Do not repeatedly reposition the popup during a drag.

Keyboard activation opens a 1 × 1 selection. Arrow keys change its dimensions,
growing the grid as necessary within the same limits; Enter accepts and Escape
cancels. Announce dimensions and the extra header to accessibility clients.
Closing restores focus to the originating editor. Keyboard and pointer acceptance
use the same portable insertion intention.

### Insertion transaction

Insert empty cells, an empty header, and an initially unspecified/left-aligned
delimiter for each column. Never insert placeholder words. Place the caret in
the first header cell in Insert mode after acceptance. In Source, that caret is
inside the first source cell rather than on its outer pipe.

Insert at the exact caret. Split surrounding prose only as needed to put the
table between valid block boundaries, preserving text on both sides and existing
list/quote ownership. At a document edge, retain or create an ordinary editable
boundary for leaving the table. New source endings use the current `fileformat`.
A new table replaces exactly the selected content when a text selection is
nonempty; never expand it to surrounding paragraphs. If that replacement
cannot preserve unselected owners, reject it without changing anything.

Insertion inside an existing table cell, literal code, or opaque/read-only
projection is unavailable; it must not manufacture a nested table or silently
move the insertion elsewhere. Capturing a picker target does not reserve mutable
state: validate the originating view, document, projection, and selection at
acceptance. An intervening source edit or view change cancels the stale picker.
Insertion and all supporting source patches form one undoable action; subsequent
typing follows ordinary Insert undo grouping.

The 20 × 50 limit belongs only to this picker. Loaded tables and structural
row/column commands have no fixed product size limit; resource limits must fail
explicitly and atomically, never truncate a table.

## Styles and WYSIWYG geometry

Add stable built-in styles **Table cell**, **Table header**, and **Table**.
Cell/header styles are structural paragraph/cell styles, not character styles.
They provide font, borders, padding, and other supported cell appearance.
Table cell inherits ordinary paragraph appearance; Table header inherits Table
cell and supplies the header treatment. Table is a container style for whole-table
placement, background, margins, padding, and perimeter appearance.

Use sparse declarations and existing style-inspector/theme ownership. Exact
default border colors, padding values, and header font treatment remain a visual
design decision; do not copy arbitrary Word theme values into the contract.
These are presentation definitions, not arbitrary source-backed formatting.
Changing their definitions uses settings undo and must not dirty Markdown.

Ignore horizontal text-alignment declarations in Table cell and Table header:
the table's delimiter cells are the authority. Show that source in the inspector
rather than providing an ineffective editable alignment control. Table's own
placement alignment is distinct: centering positions the entire table without
changing any column's text alignment. Existing inline styles resolve over the
cell/header context normally.

Always size to content, without soft wrapping, regardless of the view's wrapping
preference. Each column shares the width required by its widest cell, including
the header; each row shares the height required by its tallest cell. Within a
cell, measure the longest explicitly broken line using actual shaping, inline
styles, fallback fonts, bidi, and glyph metrics. Adding or deleting content can
grow or shrink a column immediately when its measurements are available.

The minimum **content box** is 4 em wide and one resolved font line high, measured
using the Table cell font even for header cells. Padding and borders are added
outside these minima. Actual larger text and ink must fit without clipping.
Explicit breaks increase height; ordinary text expands width. Align content at
the top of a row and apply each column's left/center/right alignment to its text.

Paint adjoining cell borders once. For conflicting cell edges, use the greater
stroke width; ties prefer the header edge, then the upper/left cell. Reserve the
resolved edge thickness before positioning each cell's text, including inherited
header borders. The Table container surrounds the complete cell grid with its own
padding and perimeter; its border does not replace a cell edge. Backgrounds follow
the existing container-before-content compositing rules. Borders and padding do
not create logical text or caret stops.

The table participates in normal block flow as one container. Its external
margins adjoin the surrounding blocks; cell paragraph margins do not become row
gaps. A following paragraph or heading measures its margin, border, and padding
from the table container's bottom, including when its syntax ends the table
without a blank source line.

An overwide table remains content-sized and horizontally scrollable. Whole-table
centering must not place its leading edge beyond the reachable scroll range;
overflow falls back to a reachable leading position. Never squeeze columns or
insert source breaks to fit a viewport. Empty cells have real caret geometry.

## Breaks, input, and keyboard movement

[GitHub supports inline HTML breaks](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#line-breaks).
Viem already interprets `<br>` in Markdown; table work must preserve and extend
that shared behavior, not introduce a table-only decoder. Recognize `<br>`,
`<br/>`, `<br />`, and other already supported equivalent passive spellings.
Retain their original bytes unless edited. Escaped markup and code-span contents
remain literal; Source displays the tag instead of breaking the visual row.

In WYSIWYG Insert/Replace within a cell:

- Inline code uses the same affinity policy as ordinary prose: an explicit
  downstream caret at its closing boundary types outside Code, including at a
  cell end. Upstream typing and Vim `a`/`A` continue Code; the toolbar reflects
  the context the next input will use.
- Enter and Shift-Enter insert a semantic hard break, serialized as `<br>`.
  They do not create a source row or a block paragraph. This is the Markdown
  adaptation of Word's in-cell paragraph/break behavior.
- Tab moves to the next cell in row-major order; Shift-Tab moves to the previous
  one. As in Word, select the destination cell's contents for replacement; an
  empty destination receives a caret. This is a single-cell content selection,
  not a rectangular Cell selection. Navigation closes the current typing group.
  Subsequent Tab/Shift-Tab continues traversal from this selection rather than
  replacing its contents with a tab; ordinary typing replaces it normally.
- Tab from the last cell appends one empty body row with the same columns and
  alignment, then enters its first cell. This also works in a header-only table.
  Row creation and the following typing use the structural-insertion undo policy.
  Shift-Tab in the first header cell stays there; it never creates a row above.
- Boundary Backspace/Delete without a selection does not merge neighboring
  cells or destroy table structure. Internal hard breaks and graphemes remain
  normally deletable. Structural widgets handle row/column deletion.
- Native arrows and selection extension use exact cell geometry and existing
  grapheme/bidi rules. Movement beyond the table reaches adjacent prose; provide
  an editable ordinary boundary at document edges when insertion requires it.

Table cell navigation takes precedence over enclosing-list Tab indentation.
Literal-next input keeps its existing precedence and representability rules.
Source has no table-specific Enter or Tab rewrite. Preserve its existing source
editing, indentation, and list-continuation rules, including structural
Tab/Shift-Tab for a table inside a list owner. Enter at a completed quote prefix
creates one empty quoted line before the retained body, just as for prose; at
the header prefix, the following quoted table remains intact. Source edits may
intentionally invalidate the table grammar. Normal/Visual Vim
commands retain their declared line, count, register, and operator semantics.
Do not silently repurpose `dd`, `o`, or `O` as row-widget commands. A complete
table/row structural deletion must be distinguished from clearing cell content
in command resolution and tested at the command entry point.

Cell text supports normal inline formatting, automatic inline Markdown,
smart-quote policy, IME, accessibility input, search, and replacement. Inline
recognition is cell-local; block-prefix recognition cannot create headings,
lists, quotes, thematic rules, or code blocks within cells. Represent a typed
pipe with the context-appropriate protection, including inside code spans.
Multiline text pasted into one cell becomes explicit cell breaks. A request that
cannot preserve literal code or protected content fails atomically rather than
silently changing semantics. Pending typing style remains view-local; IME stays
an overlay until the complete commit succeeds.

Word references for these adaptations:
[keyboard behavior](https://support.microsoft.com/en-us/accessibility/word/keyboard-shortcuts-in-word)
and [row/column insertion](https://support.microsoft.com/en-us/office/add-or-delete-rows-or-columns-in-a-table-in-word-or-powerpoint-for-mac).

### Logical order and line commands

Logical reading/search order is row-major: header cells first, then each body
row, with each cell's text in its own logical Unicode order. Each cell is a
separate text owner. Cell boundaries contribute one LF to flat logical APIs,
like paragraph boundaries; moving from the final cell of one row to the first
cell of the next contributes one boundary, not an additional blank line. Preserve
the boundary's structural kind, distinct from an in-cell `<br>` hard break.
Neither flat text nor layout gaps create editable tab/space characters between
cells. Rectangular clipboard output has its separately defined tabular encoding.

Search can address this logical stream, but a cross-cell match retains the exact
contributing cell ranges. Formatting or replacement cannot treat its bounding
source interval as selected. A replacement of a structural boundary must follow
explicit table semantics or fail unchanged; it cannot merge cells accidentally.

Ex addresses use formatted hard lines in that row-major order: each explicit
line inside a cell is a hard line. Visual line motions/operators address a line
inside the active cell, and do not select neighboring cells merely because they
share a screen baseline. Geometric Up/Down movement follows lines inside that
cell, then the same column of the preceding/following table row, retaining
desired x; at table edges it reaches adjacent prose. This is the table-specific
interpretation of visual navigation, independent of Ex/logical reading order.

Physical Source line commands retain actual source-line meaning, including the
delimiter row. Source view edits those bytes literally. In WYSIWYG, a command
that fully consumes a source table row uses structural row deletion and its
header rules; partial visual-line deletion clears only its exact cell content.
Deleting just the hidden delimiter row in WYSIWYG is unavailable because it
would destroy table interpretation without a visible content target. Counts,
cross-cell selections, registers, and replay must retain the originating line
domain and exact range set. Do not claim command support until these cases and
the complete-table deletion case have entry-point tests.

## Row and column widgets

Show a compact column widget when the pointer is over the table's top border or
the three logical pixels immediately above it. Use DIPs consistently across
display scaling. Target the column under the pointer. In left-to-right control
order, provide **Insert column left**, **Column alignment**, **Delete column**,
and **Insert column right**. The insert icons combine a plus with the indicated
arrow; delete uses a minus.

The alignment button uses the style dialog's left/center/right icon for the
current column. Its popup offers exactly those three choices and indicates the
current effective value. Choosing one changes only that column's delimiter
alignment, with minimal necessary syntax patches. It affects header and body
together. Explicitly choosing Left may record explicit left alignment even when
the previous unspecified alignment already appeared left-aligned.

Hovering either table side border or its **20 DIP gutter outside that side** shows
a row widget for that row: **Insert row above**, **Delete row**, **Insert row below**,
using plus/up, minus, and plus/down icons. Both sides target the same row actions;
prefer placing the widget beside the side that activated it. The header has no
Insert row above option; Insert row below creates the first body row.

Widgets overlay rather than reflow the document. Keep the target fixed while
moving into the widget or its alignment popup; its hit region includes the path
from the activation strip. Do not let the widget disappear under the pointer or
silently retarget on activation. At either top corner, the top-border strip
targets columns; the remaining side gutters target rows. Exact shared boundaries
belong to the following row/column, with the last outer edge belonging to the
last one. Put controls inside available screen space without covering the active
cell's editable text when an alternative placement exists.

Expose equivalent named actions through a keyboard-accessible native context
menu for the current cell. Hover cannot be the only access path. Menus, tooltips,
and accessibility labels identify the affected row or column. Refresh or dismiss
controls on scrolling, layout changes, or edits, and validate stable table/row/
column identities before an action; stale geometry cannot retarget an edit.

### Structural consequences

- Insert an empty column into the header, delimiter, and all body rows. A new
  column copies the target column's alignment. Existing cells retain identity,
  content, and source spelling wherever unaffected.
- Inserted body rows have empty cells and use existing column definitions.
  Put the caret in the first new cell after row insertion, or the new column's
  cell in the previously active row after column insertion.
- Deleting a body row removes that row. Deleting the last body row leaves a
  valid header-only table. Deleting the header promotes the first body row and
  gives it header treatment; column alignment remains unchanged. Materialize
  missing cells in a short promoted row so its header matches the delimiter.
  An overfull body row's ignored source cells cannot become extra header cells
  or be discarded. If no equivalent preserved representation is possible,
  reject promotion with an explanation that the row has extra source cells;
  leave the entire deletion unchanged.
- Deleting a header-only row or the final column removes the table. Preserve
  surrounding content and metadata; leave a valid ordinary caret boundary,
  including an empty ordinary paragraph when it was the only document content.
- After deletion, prefer the following surviving row/column, otherwise the
  preceding one. Clamp only the cell-local caret as part of the verified change;
  never substitute a current snapshot for a stale target.

Each action, including all necessary ragged-row materialization and delimiter
repair, is one atomic source transaction and undo unit. Row/column widget actions
do not implicitly populate Vim deletion registers.

## Text selection and Cell selection

Dragging starts with ordinary text selection inside the starting cell. Enter
**Cell selection** when the drag crosses into another cell or passes the text's
terminal caret into the starting cell's trailing empty area. Use the final
explicit line's logical end and bidi caret geometry; padding beyond an earlier
line of a multiline cell is not the end of the cell. Ordinary pointer jitter
must not trigger the transition. In an empty cell, a click places the caret;
dragging beyond the native threshold can select that cell.

Once entered, Cell selection stays active until release, even if the pointer
returns to the starting cell. Highlight the full rectangle between the starting
and current cells, including padding and empty cells. Preserve anchor, direction,
and active cell; support reverse dragging and bounded autoscroll. Outside the
table, clamp to its nearest edge cell. A cell rectangle cannot extend into a
second table or neighboring prose. Start a new ordinary text gesture to return
to character selection; existing keyboard document-selection commands remain
available for selections that span tables and prose.

Cell selection is a semantic rectangle of cell identities, distinct from Vim
Visual Block's display-space rectangle and from a contiguous text range. Source
does not enter Cell selection: its pointer gestures select actual source text.
Font changes, wrapping preferences, and column resizing cannot alter which
logical cells are selected.

- Bold, Italic, and other supported inline actions apply atomically to all text
  in every selected cell, with ordinary mixed-state/toggle rules. They cannot
  wrap delimiters or extend into unselected cells. Empty cells stay empty;
  formatting them must not create empty markup or persistent per-cell typing
  overrides. The active caret may subsequently have the normal view-local
  typing override after the selection is collapsed.
- Delete and Backspace clear selected cell contents, including internal breaks,
  but retain every cell, row, the header, and column alignment. Clearing an
  entire table rectangle still leaves the table. Cut uses the same clear action
  after preparing its clipboard payload. Word similarly distinguishes
  [clearing contents from deleting a table](https://support.microsoft.com/en-us/word/delete-a-table),
  but uses Backspace to remove a selected whole table. Viem deliberately makes
  both deletion keys clear a Cell selection and reserves removal for structural
  actions or whole-owner selection.
- Replacement typing clears the rectangle and inserts text into the anchor
  cell, then collapses there in Insert mode; it does not duplicate text across
  all cells. The change and subsequent typing use native replacement grouping.
  An IME commit follows that same transaction; cancellation preserves contents.
- Native Copy retains the rectangle and interaction mode. Escape clears the
  rectangle according to the originating native/Vim selection policy without
  changing text. Undo/redo follows the repository's recorded restoration policy;
  it need not restore an active native rectangle.

Ordinary nonrectangular selections retain their exact logical extent. Partial
cell deletion cannot turn into row deletion. Whole-owner deletion may remove a
fully selected table and its opaque source contents, just as other complete
owner deletion does. Unsupported cross-owner mutations fail without clearing a
convenient source hull.

## Clipboard, export, and accessibility

Copying a cell rectangle provides a structured cell matrix and a plain-text
tabular fallback, with tabs between columns and newlines between rows. Quote
fields containing tabs, newlines, or quotes using doubled embedded quotes so
cell-internal breaks are distinguishable from row boundaries. A rectangle copied
from body rows does not silently promote its first row to a Markdown header.
Source copying always preserves the selected source characters, with no visual
alignment padding added.

Structured matrix paste into a Cell selection requires matching dimensions;
a mismatch reports the size difference and leaves the selection unchanged.
Pasting a matrix at a single cell may extend the table rightward/downward as
needed, preserving existing cells outside its destination rectangle and using
the existing header. It never merges cells or replaces column alignment as a
side effect. A single text payload remains text in one cell, with line breaks
handled as above; do not guess a table solely from tabs in arbitrary prose.
Validate the whole paste before publishing changes. Counted matrix puts are
explicitly unavailable: report that the matrix must be pasted once and preserve
the document, selection, mode, history, and registers. Do not flatten a counted
matrix into text or silently ignore its count.

HTML export uses semantic table/header/body/row/cell elements and column
alignment, from the same interpreted table in either view. Export must remain
passive and escape text correctly. This does not turn HTML clipboard input into
an editable HTML-document format.

Expose table dimensions, header relationships, cell coordinates/content,
selection, and named structural actions to native accessibility. Empty cells
remain navigable. Screen readers must not report alignment gaps as spaces or
hidden delimiter rows as WYSIWYG content. Frontend accessibility limitations
remain explicit until their adapters are implemented and tested.

## Source view: align without rewriting

Recognized tables retain every original source character and physical source
line, including whitespace, optional pipes, delimiter hyphens/colons, escapes,
and literal `<br>` tags. Table rows remain unwrapped and unflowed even when soft
wrap or Flow Source Paragraphs is enabled. Draw no table borders or WYSIWYG row/
column widgets in Source. The Insert Table picker remains available.

Align corresponding structural pipe boundaries and position each cell's source
content according to its column alignment, using layout advances only. Columns
must be wide enough for the header, delimiter spelling, and literal body markup
as displayed in this view. Do not reuse WYSIWYG widths when the source text has
different metrics. Original leading/trailing cell whitespace remains present;
alignment adds geometry around it rather than consuming it.

Use Table header styling for the header's structural pipes and the delimiter
row's pipes, hyphens, and colons. Use Table cell styling for body-row structural
pipes. Explicit cell border colors also paint these source characters: outer
pipes use their left/right edge, shared pipes use the thicker adjacent edge
(the preceding cell wins ties), and delimiter hyphens/colons use the shared
header-bottom/body-top edge (the header wins ties). A header-only table uses its
header-bottom color. An unspecified winning edge color retains the character's
normal text color. These paint-only changes preserve source and text metrics.
WYSIWYG borders likewise use the resolved edge color, falling back to that
edge's cell/container text color, including the active theme's default color.
Header/body source contents retain the corresponding cell typography and
normal Source inline-markup treatment. An escaped pipe is content, not a
structural separator. Missing outer pipes or omitted body cells do not authorize
drawing invented source characters. Excess body cells remain visible, selectable
source after the aligned recognized columns, without creating WYSIWYG columns.

For example, the actual source may be:

```markdown
| Feature | Example |
| --- | ---: |
| Bold | **bold**<br>Hello, world! |
| Italic | *italic* |
```

The view adds only horizontal alignment geometry. Left/Right crosses each such
gap in one move between real text boundaries; there are no added characters,
caret stops, search matches, registers, or clipboard bytes. Clicking a gap picks
the adjacent legal source boundary with the appropriate affinity. Existing
source spaces still have their ordinary logical behavior.

The request's pasted example collapsed these physical rows into one line. A
literal single-line sequence such as `| a | b || --- | --- || c | d |` is not a
table and must stay one line; never infer or insert missing source breaks for
presentation. Explicit Insert Table and structural edits can create required
source lines, but opening, drawing, or switching views cannot.

An edit that invalidates the table grammar returns the affected region to normal
Source layout without repairing the user's literal input. A later valid edit
restores alignment. Switching views preserves exact bytes, anchors, cell-local
locations where recoverable, and viewport position through checked provenance.

## Preservation, ownership, and bounded work

Table, row, column, and cell structure belongs to the portable document model;
command interpretation issues semantic intentions and layout consumes snapshots.
Native controls supply events and paint the shared geometry. Neither platform
may own a separate table editor, selection model, or undo stack.

Source bytes remain authoritative. Do not normalize whole tables to align pipes
or edit one cell. Preserve unrelated spellings, encoding, BOM, mixed endings,
outer-pipe choices, whitespace, and final-terminator presence. Missing body cells
are editable semantic empties; materialize only the slots needed by an edit.
Ignored excess cells and unsupported syntax remain anchored in source. Widening
a table must not accidentally promote formerly ignored cells into visible data;
insert required empty slots before them. Narrowing, moving, or deleting structure
must likewise preserve unselected contributors or reject the transaction.

Prepare source patches, reproject, and verify cell contents, table shape,
alignment, owners, and unaffected boundaries before atomic publication. All
entry paths, including paste, IME, accessibility, Vim commands, and toolbar
actions, use that route. Cancelling or rejecting an edit changes no source or
success-dependent state. Undo/redo restores exact original artifacts rather
than regenerating Markdown from the grid. Stable identities carry selections,
carets, and viewports through row/column edits; do not retain naked row indexes
as durable targets.

Exact content sizing creates an explicit dependency across one table: an
offscreen cell can be the widest in a visible column. Cache width contributions
and row metrics so ordinary edits do not rescan the table. Deleting or narrowing
the widest cell must recover the next maximum, not leave permanently inflated
widths. A column-width change moves later columns but does not require reshaping
unchanged text; paint-only changes reuse metrics.

Cold discovery for arbitrarily large tables must remain progressive, bounded,
cancellable, and visible-first. Until all relevant measurements are ready,
column widths may be provisional; completed geometry must equal a fresh exact
content-size result. Keep that distinction explicit internally. Never install
stale widths or hit-test with geometry from another snapshot, and obtain exact
local caret/hit geometry before executing an input intention. Width refinement
preserves the editing baseline and anchored viewport with only necessary reveal.
This temporary discovery policy is the exception to immediately exact sizing,
not permission to retain fixed widths or block editing on offscreen work.

No table operation justifies full-document relayout per keystroke. Shape huge
cells incrementally, bound caches/queues and native allocations, coalesce obsolete
work, and prioritize the active view. Source and WYSIWYG measurements have
separate dependencies while sharing source and semantic identities.

## Acceptance coverage

The feature is complete only with matching portable behavior and native event/
rendering checks on macOS and Windows. Register integration tests under
`tests/all/main.rs`; keep the compatibility inventory and feature demo honest as
implementation lands.

| Area | Required cases |
| --- | --- |
| Parsing | GFM table examples 198–205; optional pipes; short delimiter runs; every alignment; mismatched headers; header-only tables; empty, missing, excess cells; escaped pipes/backslashes/code; entities; block termination; list/quote ownership; literal code/HTML contexts. |
| Insertion | Click-open and drag-open; native jitter threshold; cell press/drag/release; hover; outside/Escape/capture loss; above/left cancellation; right/bottom independent clamping and mixed corners; growth to 20 × 50; small screens; keyboard/accessibility; stale originating targets; counts excluding the header. |
| Geometry/styles | Minimum sizes measured from Table cell; wider headers; multiline/mixed-font/bidi/emoji cells; shrinking after maximum deletion; shared borders; per-column alignment; theme/zoom/fallback changes; horizontal scrolling; blank-cell carets. |
| Editing | Inline formatting and protected pipes; Enter/Shift-Enter and `<br>` variants inside/outside tables; literal Source/code contexts; repeated Tab traversal/last-cell append; boundary deletion; row-major search/Ex and cell-local visual lines; physical row commands; IME commit/cancel; all row/column insert/delete cases including ragged header promotion and last-column removal. |
| Selection/clipboard | Text-to-cell transition; multiline/RTL/empty cells; reversed rectangles; autoscroll; mixed formatting; clear versus structural deletion; native Copy preservation; exact replacement scope; matrix paste/mismatch; quoted tabular fallback; register and command policies. |
| Source presentation | Unequal spelling lengths; no synthetic copy/search/caret content; gap hit testing; optional/missing pipes; visible excess cells; no soft wrapping/flow; literal `<br>`; malformed edits; both view switches. |
| Preservation | No-op byte equality; minimal patches; ragged/excess cells; opaque content; mixed endings/encodings/BOMs; final terminators; reopening edits; exact undo/redo; cancellation/stale failures; multiple views. |
| Scale | More than 20 columns and 50 rows; very large tables and cells; cold visible interaction; edits near/offscreen width maxima; work-count bounds; cache eviction; rapid resize/style/edit changes; stale background results; native memory/latency. |

Word's [table insertion](https://support.microsoft.com/en-us/word/training/insert-a-table)
and [content sizing controls](https://support.microsoft.com/en-us/word/resize-a-table-column-or-row)
are interaction references. Viem specifically uses the requested 10 × 10 growing
picker and mandatory unwrapped content sizing rather than claiming every Word
table option is representable in Markdown.
