# Blocking editor and Git

`blocking-viem <file>` starts Viem or opens the file in its existing process, then
waits until every pane and window showing that shared document has closed.
Other open files do not keep the caller waiting. Relative paths use the caller's
working directory, and paths with spaces must be quoted.

On macOS, the helper ships inside the application bundle. For an app installed
in `/Applications`, configure Git with:

```sh
git config --global core.editor '"/Applications/Viem.app/Contents/MacOS/blocking-viem"'
```

For a development build, substitute the absolute path to
`.build/Viem.app/Contents/MacOS/blocking-viem`. A symlink to that helper on `PATH`
also works; leave the actual executable beside `Viem` in the bundle.

`make release` and `make run` also build and package the release helper at
`.build/release-app/Viem.app/Contents/MacOS/blocking-viem`. This location survives
subsequent debug builds. A shell redirector should use `exec` and forward `"$@"`
to that helper, preserving Git's working directory, arguments, and exit status.

On Windows, keep `blocking-viem.exe` beside `Viem.exe` and its runtime files.
The build script publishes the helper as a native executable. Neither it nor
the Release editor requires a separately installed .NET runtime.
Put their directory on `PATH` and run:

```powershell
git config --global core.editor 'blocking-viem'
```

Git supplies the message filename. Use `:wq` to save and close the current view.
Saving with `:w` keeps the caller waiting. If another split or window still shows
the document, closing this view also keeps it waiting. Reloading, changing view
mode, and cancelling a close dialog do not end the operation. Concurrent callers
for the same document all complete when its last view closes.

Git's commit, merge, squash, tag, notes, and branch-description message filenames
automatically select Git Commit syntax. View > Code > Auto shows the detected
language; explicit format or language choices take precedence.

Use `:cq` or `:cquit` to cancel a Git commit. This abandons **all open documents**
without saving and exits Viem with status 1, which is returned to every pending
blocking caller. `:7cq` or `:cq 7` supplies another status; `!` has no additional
effect. On macOS, process exit statuses follow the usual low-eight-bit convention.
Already saved changes stay saved. Explicit status zero reports success.

Ordinary discard or `:q!` reports success using the file's existing disk contents;
use `:cq` when Git must abort. Launch/open failure or a GUI crash reports failure.
Only startup has a timeout; human editing has no timeout. Closing or killing the
blocking helper does not close the editor or save its document.

The completion follows the document even if it is renamed or saved under a new
name. Git still reads the filename it originally supplied, so save a commit
message to that original file before closing it.
