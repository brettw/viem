Mouse wheel scrolling seems to go the wrong way. Make sure we're following the direction properly
since the system settings can change the direction for the wheel.

I tried loading and saving commands like ":e foo.txt" or ":s bar.txt" and they didn't work. The
":e" command should replace the current window (assuming it's been saved). Let's define ":E ..." to
open in a new window.

I tried ":pwd" and it didn't work. For this, we need to define a way to display this data. When a
command like this produces output, keep the input line open and replace it with the read-only value.
Add an "x" on the far right to explicitly close this bar in this state. When I type another ":"
command, go back to normal (replace the contents with my typing, close automatically).

When the input line is active (I type ":" in normal mode) I should be able to mouse select and
select with shift+arrow keys. I should be able to use the system accelerators for copy-paste
(command key and menu commands) to copy and paste from this area.

In our HTML format we define "--evim-bold" and "--evim-base-weight". Use bold and italic tags for
these simple items when possible and use style for more complex ones like font name.

When converting from markdown/html to plain text, it currently keeps the original text which is
correct. When converting between html and markdown, we should convert and convert the styling into
the correct tags for that format. It will be lossy in some cases which is OK. In this case, output a
warning to the status output line (described above for "pwd") with a warning that some information
has been lost with this conversion. Same with encoding conversions to latin1.

We should define a context menu (right click, other standard access on Mac). It should have the
standard system items like cut/copy/paste.

Define a "code block" (block style) and "code" (char style) used for html pre/code tags and markdown
back-ticks (inline and block. Default it to the system monospace font, dark green.

We should define how settings work. In "~/.evim/config.json" we should store our settings. Serialize
and deserialize the app-wide settings like the themes there.

We should define default style sets in that directory: text_style.json, html_style.json,
markdown_style.json, and "rtf_style.json". Replace the "Format > Document style" menu with a new
"Style" submenu. In there we should have "Edit document style" and "Save as default XXX style" where
XXX is "text", "markdown", etc. This writes the current style configuration to the default file.

When loading a document, load the default style. Then apply any document styles (like from html) on
top. We do not need to expose this to the user. Just consider anything inherited from the default
style as unset when writing the document (i.e., if I don't explicitly set a font, a written html
file should not contain that either).

Implementation status: all items above are implemented. Computer-use checks and
automated validation are recorded in [docs/todo3-validation.md](docs/todo3-validation.md).
Compatibility candidates deliberately not adopted are listed in the final section
of [GAPS.md](GAPS.md).
