# WYSIWYG editing rules and error audit

> Historical audit: references to HTML/RTF editing and general format conversion
> describe retired implementations. Current editable formats are Text, Code,
> and Markdown; only Markdown Source/WYSIWYG switching remains.


Ordinary deletion and replacement are defined in visible text coordinates.
Hidden markup is the adapter's responsibility; it must not make a valid text
selection undeletable. The source transaction still verifies its projected
result before publishing any changes.

## Replacement formatting

The first selected character in document order supplies replacement character
formatting, skipping leading paragraph separators. Selection direction does not
change this choice. Capture that
context before removing the selected text, and retain it for the first typed
input and subsequent typing. Explicit pending character formatting overrides
the inherited context.

For `baseparagraph <b>bold</b> baseparagraph`:

| Selection | Replacement character formatting |
| --- | --- |
| Starts before bold, ends after bold | Regular |
| Starts at bold, ends in following ordinary text | Bold |
| Exactly the bold run | Bold |
| Interior of the bold run | Bold |
| Starts before bold, ends at the bold run's end | Regular |

The same rule includes actual HTML link context, including its destination.
An entirely consumed interior link does not lend its appearance or destination
to a replacement beginning outside that link. Inline tags emptied by replacement
cannot override the replacement's captured typing context. Pre-existing empty
styled spans retain their ordinary caret-insertion behavior.

Replacing a whole paragraph retains that paragraph's style and direct paragraph
declarations, including when the replacement consumes the entire document.
Joining paragraphs retains the first paragraph's style. A selection containing
only paragraph separators uses its pre-edit insertion context, excluding a
neighboring hyperlink; empty paragraphs use their own typing context/defaults.

## Input recovery policies

- HTML and RTF use native escapes for characters outside the file's encoding.
  Markdown prose also uses numeric Unicode references where their interpretation
  is exact. Markdown code and source/literal modes remain literal and never
  substitute entity syntax for a requested character.
- External plain-text paste normalizes CRLF and CR to logical breaks. Private
  register payloads retain explicit break semantics. HTML paste replaces NUL
  with visible `␀` without a modal question or changing the clipboard. Exact
  command-prompt/register/literal-input semantics remain distinct from paste.
- Inserting immediately after a dangling UTF-16 byte replaces only that byte
  with properly encoded U+FFFD before appending the input. The visible diagnostic
  stays present. Repair and insertion are one transaction, and Undo restores the
  exact malformed source. Opening, saving, and edits elsewhere preserve those
  bytes. The transaction exposes a repair warning; ordinary typing currently
  does not forward that warning to frontend notifications.
- Unsupported embedded objects remain atomic, with positions only before and
  after the object. Whole-object deletion/replacement works; no implicit
  conversion to text or object-interior editing is introduced.

## Structural repair rules

- HTML displayed-row deletion uses the same whitespace protection as ordinary
  range deletion. A retained space exposed at a paragraph edge becomes a
  protective nonbreaking space, following the existing HTML editing policy.
- An edit affecting only part of a multi-character entity rewrites its source
  contributor while retaining the unselected visible characters.
- If deletion exposes a leading newline inside HTML preformatted content, use
  an explicit break so HTML's initial-newline suppression cannot drop it when
  the file is reopened.
- Deleting the body of an HTML parent list item retains its visible empty
  paragraph and surviving nested list through an explicit empty paragraph.
- An emptied paragraph retains its inside-element insertion boundary, so typing
  into it behaves the same before and after reopening the file.
- Markdown literal text must remain literal when an edit exposes punctuation
  that would otherwise become a link, list marker, or other markup.
- Hidden comments and opaque content remain intact unless their complete visible
  atomic object is selected. Paragraph joins keep the first paragraph's style;
  retained inline formatting stays attached to retained text.

## Audit boundaries

The audit exercises legal visible ranges, including Unicode graphemes, across
HTML, Markdown, and RTF. It covers deletion and replacement with ordinary text,
spaces, and hard breaks; inline styles and links; paragraph/list boundaries;
entities; recovered HTML; and opaque object, field, and comment boundaries.
Native tests cover macOS text input, IME replacement, and deletion of the final
wrapped row through Visual Line mode.

The initial audit did not sufficiently exercise native line-selection command
state. The last list item in `<ul><li>Now ist the time </li><li>For all good
men</li></ul>` exposed this gap: converting a native line selection to its
replacement extent left a remembered selection endpoint at the former EOF.
History preparation then rejected that stale offset after the document became
shorter. The fix retains the original selection identity so the existing change
map rebases it before history publication. Hidden closing tags were not part of
the visible selection.

The expanded command audit compares 2,898 contiguous line-selection deletions
against equivalent plain-text commands, across both directions, Visual and native
selection policies, and Normal/Insert return modes. It includes empty neighbors,
first/middle/last list items, nested lists, continuation paragraphs, preformatted
blocks, block quotes, and atomic objects. Separate tests enumerate legal native
character selections and caret Delete/Backspace positions, and exercise disjoint
Visual Block ranges. Every successful edit is reopened independently and checked
through exact source undo/redo. Native mouse-event tests triple-click every line
in the reported example and related containers with both physical Delete keys.

Additional repairs found by this pass:

- Half-open line selections exclude unselected empty paragraphs at their edges.
  Preformatted and quote owners are removed only when their contained paragraphs
  are selected; retained paragraphs keep their enclosing style context.
- Empty anonymous HTML paragraphs receive a local explicit paragraph owner when
  their surrounding boundaries must survive. This includes div, definition-list,
  and implicit list-prefix bodies. Original container attributes remain intact.
- Whole-document clearing removes an atomic object's fallback text and nested
  opaque content with the object. Outside comments and hidden document metadata,
  including nested templates, remain preserved.
- Markdown deletion preserves both boundaries around an emptied continuation
  paragraph, including quoted lists, encodings, and adjacent batch deletions.
  Deleting an item's first paragraph promotes its surviving continuation body
  while retaining the item marker. Backspace at a continuation paragraph joins
  it to the preceding visible paragraph; removing an actual item label also
  removes that item's hidden continuation indentation.

Native Copy now uses a dedicated portable intention. Keyboard, menu, and Copy
selectors retain selection direction, active endpoint, mode, and geometry while
exporting plain and rich clipboard data. Pending Vim commands and mappings are
left untouched. Ordinary Vim yanks retain their existing
selection-ending behavior. Tests cover character, line, and block selections,
repeated Copy, source views, and both native frontends' shared command path.

The cross-format matrix verifies 5,512 deletion/replacement cases by committing,
reopening, comparing the requested visible text, and undoing to the exact source
bytes. Another 148 Markdown replacements across inline hard breaks check retained
character traits and link destinations as well as text. Encoded fixtures cover
UTF-8, UTF-16LE/BE, source patch locality, and continued typing. The Markdown
repair has a 10,000-paragraph regression requiring local decoding and no complete
formatted-text materialization.

The following checks remain necessary and are not ordinary deletion failures:

- Stale document revisions, wrong-document ranges, invalid grapheme boundaries,
  overlapping edits, and explicitly read-only buffers cannot be edited through
  an invalid request.
- Exact literal character insertion can still require a format or encoding
  change where no equivalent escape exists: for example, emoji in Latin-1
  plain text or Markdown code, HTML NUL outside the paste policy, and literal
  carriage returns under a conflicting line-ending interpretation. Ordinary
  input does not silently change encoding or substitute a different character.
- Opaque embedded objects have before/after positions and are editable as whole
  objects. Editing their unsupported internal content requires a separate
  object editor and persistence policy.

The former separator-inheritance question is resolved: replacing the separator
and `B` in `<p><b>A</b></p><p>B</p>` inherits regular character formatting from
`B`, while the joined paragraph retains the first paragraph's paragraph style.

An additional foreground-model limitation was found while testing paragraph
defaults. In `<p style='color:red'>A</p><p>B</p>`, replacing the separator and `B`
should retain the first paragraph's red default while giving new text `B`'s
theme-default foreground. The current sparse color model has only an inherited
value or an explicit RGBA color, so it cannot express that reset independently.
Hard-coding black would break theme adaptation, and removing the paragraph color
would restyle retained content. This case remains a followup: introduce an
explicit default-foreground declaration, preserve it through style resolution,
HTML/RTF serialization, clipboard and the native style API, and test theme changes
and reopening. Background resets use the existing transparent-color declaration;
that is visually equivalent to no character background while preserving the
paragraph default.
An absent character language has the same modeling limitation when joining a
paragraph with an explicit `lang`: inherited absence cannot currently override
that default. The explicit-reset followup should cover language alongside the
theme-default foreground, without replacing a language with a guessed value.

Candidate-verification failures remain internal consistency errors. Suppressing
verification or committing a candidate with the wrong text would hide an editor
defect and risk data loss. The former message describing this as an inability to
preserve the format has been removed from both native frontends.

## Historical scope

The HTML editing mode and its replacement scope index have been removed.
HTML files now use Code; passive Markdown HTML and clipboard import retain
separate semantic parsing. Earlier HTML editing results above are historical.
