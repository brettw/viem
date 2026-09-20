# Windows Release startup

Measured on 2026-09-19 on Windows 10 x64 build 19045, Intel Xeon E5-1650 v3
at 3.50 GHz, .NET SDK 10.0.401/runtime 10.0.12, Windows App SDK 2.1.3 and
Win2D 1.3.2. These changes affect the Windows frontend; the Rust core and Mac
frontend are unchanged.

## Result and measurement method

Five alternating launches of each build, with compilation and tests stopped:

| Build | Empty median | C# median | Empty mean | C# mean |
| --- | ---: | ---: | ---: | ---: |
| Original Release | 1,007.10 ms | 1,235.57 ms | 1,005.37 ms | 1,264.73 ms |
| ReadyToRun alone | 771.26 ms | 1,008.54 ms | 772.48 ms | 1,007.73 ms |
| Final Release | **740.44 ms** | **982.28 ms** | 755.71 ms | 963.01 ms |
| Median reduction | **266.66 ms (26.5%)** | **253.29 ms (20.5%)** | | |

The baseline is commit `d46e85e2b22de4c35ddd25a2bc2e6437bcd3b811`, with its
original Release output preserved before modification. The C# fixture is a
frozen copy of that commit's `src/win/Shell/EditorWindow.cs`: 31,646 bytes,
444 lines, SHA-256
`96dd5fc1ffbcd45ae2862b3debe63fa73d0f0e9b0851c4bff3a0f799859ddcbd`.
This is a representative file, not a claim about an unspecified user document.

Each process gets an isolated profile containing a copy of the same
`config.json`; the user's profile is not modified. The measured canvas is
1258.67 by 858 DIPs. Startup opens the fixture through the actual command line.
The script verifies that canvas dimensions, top position, first baseline and
canvas position stay unchanged for the following second. Empty startup does
not load font-picker lists or enumerate font-face descriptions.

The endpoint is completion of the first editor Draw callback, measured from
process creation. It excludes compositor presentation and the later one-second
trace collection window. These are repeated launches with ordinary OS caches,
not reboot/cold-disk measurements. The first launch after the final rebuild was
**1,004.50 ms empty**, reported separately. Earlier first launches into newly
copied output folders were about 3.0-3.8 seconds, including the unmodified
baseline; they must not be compared with repeated launches. Tracing localized
much of that first-folder delay to runtime/WinUI initialization but did not
identify its loader, cache or security subcomponents.

Final individual process times, in launch order:

| Scenario | Original (ms) | Final (ms) |
| --- | --- | --- |
| Empty | 1029.23, 1018.10, 974.34, 1007.10, 998.09 | 816.98, 732.76, 787.79, 700.57, 740.44 |
| C# | 1235.57, 1253.44, 1393.54, 1221.98, 1219.13 | 1003.71, 976.51, 1000.27, 852.27, 982.28 |

[Timing data and the complete representative traces](windows-release-startup-performance.json)
retain the observations used below. Run-to-run variation is material; small
scope differences are evidence about work removed, not precise independent
end-to-end savings.

A separate five-run alternating confirmation copied the saved Code and Markdown
style defaults as well as `config.json`. This matches the installed profile more
closely. The Code style selects `RecursiveSansLnrSt-Regular` at 17 DIPs; it is
preserved, not substituted to obtain a faster benchmark.

| With saved settings and styles | Original median | Final median | Reduction |
| --- | ---: | ---: | ---: |
| Empty | 1,076.15 ms | **744.23 ms** | **331.92 ms (30.8%)** |
| C# | 1,405.44 ms | **1,113.50 ms** | **291.95 ms (20.8%)** |

The five final C# times were 1078.44, 1113.50, 1174.29, 1451.54 and 1078.82 ms;
the 1451.54 ms observation remains included (mean 1179.32 ms). The final empty
times were 744.23, 740.51, 747.89, 760.15 and 740.54 ms. The full-profile
observations and representative traces are also retained in the JSON artifact.

## Changes and time removed

| Implemented change | Empty startup saved | C# startup saved |
| --- | ---: | ---: |
| Publish Release as ReadyToRun, including managed framework projections | 235.84 ms | 227.03 ms |
| Remaining application changes together | 30.82 ms | 26.26 ms |
| Total, using the same cohort's medians | **266.66 ms** | **253.29 ms** |

Release now uses `dotnet publish` at the existing executable path. Previously,
`dotnet build -c Release` produced optimized IL that still needed substantial
compilation during startup. The final Viem assembly's native header was checked
to confirm that it really contains ReadyToRun code. Debug builds keep their
normal behavior. [Microsoft describes the startup benefit and remaining JIT
cases in its ReadyToRun documentation](https://learn.microsoft.com/en-us/dotnet/core/deploying/ready-to-run).

The application changes in the second row are:

- Read and validate initial preferences on a worker during WinUI initialization.
  The representative reads take 27.50 ms empty and 29.02 ms for C#, while the
  UI joins the completed task in about 0.02 ms. The worker creates no controls
  and disposes its temporary core validation document before handoff.
- Generate JSON metadata for launch arguments, recovery records, preferences,
  and font features. Empty feature dictionaries use the literal `{}`. This
  avoids reflection and unnecessary allocation on first use. Preference updates
  retain unknown keys and merge external changes as before.
- Notify the recent-file menu separately, avoiding a full settings/theme refresh
  on file open. External settings changes merged during that write still notify
  editors. The representative recent-file scope falls from 21.40 to 14.65 ms;
  that scope includes serialization and is not an additional end-to-end saving.
- Generate the WinRT wrapper for the glyph renderer and cache native face
  metadata and exact glyph ink bounds across shaping fragments. For the C#
  viewport, metadata reads fall from 63 to 1 and native glyph-bound queries from
  2,502 to 77, with 4,209 cache hits. Representative cluster processing is
  62.79 ms with ReadyToRun alone and 57.01 ms in the final build. The cache is
  bounded to 32 faces and 1,024 glyph entries per shaper, uses native face
  equality, and is cleared by metrics/device changes and disposal. Worker
  shapers have separate caches.

The individual scope observations overlap and were not each measured with a
complete independent ablation. Only the two build-stage rows above should be
added to obtain the reported startup savings.

## Where the remaining time goes

These are disjoint intervals from the actual median run for each scenario.
They sum to the measured process lifetime through the first draw; tiny
activation-dispatch bookkeeping is included with activation.

| Phase | Empty | C# | Work included |
| --- | ---: | ---: | --- |
| Process creation through managed entry | 97.77 ms | 117.67 ms | OS loader, CLR, dependencies and initialization before Main |
| WinUI Application.Start | 87.26 ms | 89.15 ms | COM setup, native framework startup and callback dispatch; preferences overlap this interval |
| Application resources and launch dispatch | 61.76 ms | 65.66 ms | Application resources, remaining framework launch dispatch, font-index preparation and instance broker |
| Native window, chrome, controls and menus | 231.72 ms | 217.26 ms | HWND/control construction, titlebar, saved placement, theme and menu setup; editor pane counted separately |
| Window activation | 30.49 ms | 41.96 ms | Native activation and immediate dispatch |
| Document open/configuration and editor pane | 62.49 ms | 144.68 ms | Empty pane, or file/recovery checks, read, parse, defaults, recovery ownership, pane and recent-file update |
| WinUI arrange/resource initialization and dispatch | 103.38 ms | 104.11 ms | Layout of the native control tree and Canvas resources before CreateResources/Attach |
| View provider and initial text layout | 42.01 ms | 157.06 ms | DirectWrite setup, shaping, geometry, and bounded background-layout scheduling |
| Status/input attachment | 16.35 ms | 20.82 ms | View notifications, status, input host, focus and initial refresh |
| Draw dispatch and submission | 7.22 ms | 23.92 ms | Wait for Draw plus drawing callback, not physical presentation |
| **Total before rounding** | **740.44 ms** | **982.28 ms** | |

Inside the C# document/pane interval, the read takes 16.67 ms (including async
dispatch), actual source construction/detection takes **2.07 ms** on a worker,
configuration/recovery ownership takes 42.40 ms, pane construction 60.66 ms,
and recording the recent file 14.65 ms. About 8.23 ms is the surrounding path,
recovery-candidate and dispatch work. Reading/parsing the moderate file is not
the primary delay. Recovery ownership and durable writes are preserved.

Inside view creation, the provider takes 4.31 ms and initial layout 151.53 ms.
Foreground shaping accounts for 137.04 ms of that layout: font setup 11.63 ms,
native glyph capture 55.43 ms, cluster extraction 57.01 ms, plus other shaping
bookkeeping. It shapes 4,286 characters for the visible region, not the entire
file. An empty view still needs font metrics and a legal caret line.

Process-wide JIT CPU time is 196.94 ms empty and 266.13 ms for C#. This is an
overlapping CPU measurement, **not another wall-time phase**. ReadyToRun does
not precompile every generic/interop path.

The intervals account for the remaining elapsed time, but do not prove that
every millisecond is irreducible. Most is framework initialization/control
construction, required native text work and dispatch in the current WinUI
design. Further large reductions require dependency/interop changes or a
different shell construction strategy, rather than faster C# file parsing.
There is no unexplained large document-wide layout pass in these traces.

With the saved styles, the median runs break down as follows. The resource/
dispatch interval also includes launch-time style loading when applicable.

| Phase with saved settings/styles | Empty | C# |
| --- | ---: | ---: |
| Process creation through managed entry | 123.85 ms | 111.18 ms |
| WinUI Application.Start | 95.80 ms | 98.25 ms |
| Application resources and launch dispatch | 69.92 ms | 87.57 ms |
| Native window, chrome, controls and menus | 212.10 ms | 197.46 ms |
| Window activation and immediate dispatch | 24.03 ms | 36.28 ms |
| Document open/configuration and editor pane | 52.87 ms | 135.08 ms |
| Native arrange/resources, style loading and dispatch | 99.42 ms | 87.44 ms |
| View provider and initial text layout | 40.93 ms | 326.78 ms |
| Status/input attachment | 17.90 ms | 17.29 ms |
| Draw dispatch and submission | 7.41 ms | 16.17 ms |
| **Total before rounding** | **744.23 ms** | **1,113.50 ms** |

The saved font makes native shaping noticeably more expensive: initial layout
is 320.65 ms, including 29.06 ms to resolve its installed PostScript face and
184.55 ms in native glyph-capture calls, versus about 55 ms for capture in the
settings-only C# sample. The cache still limits that viewport to one metadata
read and 176 native glyph-bound queries, with 4,110 hits. These measurements
identify font resolution/shaping as a remaining cost; they do not justify
changing the chosen font, disabling shaping features or claiming that all
possible future layout improvements have been exhausted.

## Experiments not retained

- Composite ReadyToRun did not show a repeatable improvement over ordinary
  ReadyToRun and was left disabled.
- Background preparation of Win2D's shared device shifted time between window
  construction and resource initialization. Five-run means with it were
  758.31 ms empty / 970.99 ms C#; without it they were 755.71 / 963.01 ms.
  These differences are small relative to variation, so the extra worker and
  retained device were removed.
- NativeAOT built successfully, but the actual editor could not create its first
  view. Win2D's `ICanvasTextLayoutMethods.get_LineMetrics` threw
  `NotSupportedException: Cannot handle array marshalling for non blittable
  type 'Microsoft.Graphics.Canvas.Text.CanvasLineMetrics'` from
  `WinRT.MarshalNonBlittable<T>.DisposeAbiArrayElements`. NativeAOT is therefore
  not enabled. A future attempt must update or replace that projection and run
  the native UI/rendering suite; a successful native compilation alone is not
  sufficient. [CsWinRT documents its AOT requirements](https://github.com/microsoft/CsWinRT/blob/master/docs/aot-trimming.md).

## Validation and reproduction

Final Release and optimized diagnostics builds succeeded. All 142 generated ABI
declarations match the headers. The focused native suites passed **114 checks**
for startup, preferences/recovery, multilingual rendered pixels, large documents
and cache invalidation, and **121 checks** for style/settings and live colors.
All final startup runs passed the geometry and lazy-font-discovery assertions.

The broad UI suite is **not consistently green**. Separate attempts failed at
the read-only-output menu target assertion and live Code-color rollback/update
assertions. The unchanged baseline passed 338 checks in one comparison, and the
focused final suites pass, so this investigation does not establish whether
those broad-suite failures are preexisting or caused by timing changes. No
assertions were weakened. Relevant final reports are
`result-c8cca4d525a14c9c8b0708a94552e4b7.json` (114) and
`result-55a24dcc6ee44583900c257cc65b62ef.json` (121) under
`target/windows-validation`.

```powershell
./scripts/build-win.ps1 -Configuration Release -Offline
./scripts/test-win-startup.ps1 -NoBuild -Runs 5 -Label empty
./scripts/test-win-startup.ps1 -NoBuild -Runs 5 -Label csharp -ProfileDocument path/to/file.cs
# Optional: -ConfigFile path/to/config.json and -Executable path/to/preserved/Viem.exe
# Or: -ProfileDirectory path/to/profile (copies settings, startup commands and style defaults)

$env:VIEM_TEST_STARTUP_ONLY = '1'
./scripts/test-win.ps1 -Optimized
Remove-Item Env:VIEM_TEST_STARTUP_ONLY
$env:VIEM_TEST_STYLES_ONLY = 'all'
./scripts/test-win.ps1 -NoBuild -Optimized
Remove-Item Env:VIEM_TEST_STYLES_ONLY
```

The first dependency restore must be online, including the Crossgen2 package;
`-Offline` is for already-restored dependencies. Preserve a baseline output
before rebuilding, alternate builds/scenarios, and record first launches of
new output separately. [Microsoft's startup guidance](https://learn.microsoft.com/en-us/windows/apps/develop/performance/app-startup-performance)
also recommends measuring meaningful startup milestones with Release builds.
