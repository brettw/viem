We should have two HTML modes to match the two markdown modes. The current HTML mode should be "HTML
WYSIWYG". Then we should add a "HTML Source" mode which shows the exact tag source in the document.
Rename our current "markdown" mode to be "Markdown Source" (shows markdown tags).

In HTML source mode we should show all characters of the source document and keep them in sync with
with the visible styles. When I edit the tags, we should apply the styling accodingly and
vice-versa.

When in HTML source mode we should expose some internal styles. These styles will not be selectable
by the user but will be applied to HTML tags. We will have internal character styles for HTML
brackets, tag names, the attribute keys, attribute values, the "=" sign in attributes, entity names
(&amp;). Internal style names should start with "*" so "* HTML Attribute value". Internal attributes
should not appear in the style picker menus, only in the style editor dialog (so they van be
configured). The style picker should reflect the current surrounding style, ignoring the builtin
autoapplied style. Define for these internal styles some appropriate colors by default (everything
else should be inherited from the underlying style.

Also define an auto HTML character style for uninterpreted text like the contents of `<script>` or
`<style>` tags.

In HTML source mode, if I start typing "<" then a strict parsing of the document will become
corrupted. We should avoid that. If I type "<" we should automatically insert ">" AFTER the cursor
to keep the parsing valid, and if I complete typing a start tag like "<b>" we should automatically
append the corresponding end tag "</b>". So typing literally "<b" should insert "<b|></b>" with "|"
indicating the caret. Keep the automatically inserted text as a known entity so we can change it as
I type (following it with "r" should change the insertion to "<br|>" with "|" being the caret). We
shouldn't automatically fix code we didn't automatically insert. Only track the most recent
automatic tag insertion, and clear the auto annotation if the user manually changes any of the
automatically inserted text.

In HTML WYSIWYG mode, typing  e.g. "<" should insert the entity so it doesn't change the parsing.
Same for other related special characters.

In settings make a new section "Editing". Add a "smart quotes" option there and default to off. When
on, we should detect cases where the literal quote character is not syntactically required (so
attribute values in HTML Source mode, maybe something similar in Markdown Source mode (?). If at the
beginning of a line or following a hyphen (define some other rules you can identify as being used by
common edits), they should be converted to the opening quote "6" style. When following a letter or
end punctuation like "," or ".", it should default to the closing "9" style of quote.

The icon for underline ("U") should have a line uneder the U.

When I select a paragraph style from the menu, it should apply that style to the current paragraph.

We should support double-click for word selection and triple-click for line selection.

I can't seem to apply styling in RTF mode. I would expect things like bold to be supported there.

I should be able to type things like option-i in insert mode with no selection and it should apply
to my subsequent typing. In other words, it should insert an italic region and place my cater in the
middle of it. In some modes like markdown, this may produce invalid markdown like four asterisks in
a row. This "current insertion" style should be temporary and can be removed if I move the cursor
away without typing any contents. Think carefully about the design here and make it match other word
processors and implemented cleanly.
