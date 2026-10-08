# Bundled fonts

These original desktop font files are shared by both native frontends. App
packaging copies this directory to `Resources/fonts`, including the upstream
licenses, copyright notices and documentation. Fonts remain app-local; Viem
does not install them for other applications or embed file paths in saved styles.

- `recursive/Recursive_VF_1.085.ttf`: the supplied Recursive 1.085 variable
  TrueType font, with Monospace (`MONO`), Casual (`CASL`), Weight (`wght`),
  Slant (`slnt`), and Cursive (`CRSV`) axes and 64 named instances. It replaces
  the static collection. The original `LICENSE.txt` and `README.md` accompany
  the font.
- `flightline/FlightlineCode-{Regular,Italic}-VF.ttf`: the supplied upright and
  italic Flightline Code variable fonts. Each has a Weight (`wght`) axis from
  200 to 700 and six named instances, replacing the 12 static files. The
  original `OFL.txt`, `README.md` and
  `TRADEMARKS.md` retain the distribution's copyright and licensing information.
  Saved styles use the portable family and subfamily names with independent
  base weights and variation coordinates. Lookup does not rewrite configuration.

Font binaries are copied byte-for-byte. Before creating editor UI, macOS checks
the available PostScript names and uses installed faces when they cover every
face in a packaged font file. This is a name-only match, without comparing font
versions or bytes. Files with missing faces are registered with Core Text at
process scope. Windows reads bounded metadata from each single-face variable
font and checks indexed installed-family candidates before opening an app-local
Win2D font set. It reuses an installed design only when its original identity and
`name`, `fvar`, and `STAT` tables match. Missing, incompatible, static, collection,
or unreadable candidates keep the normal bundled-file path. Each design is
independent, so an installed upright face cannot hide a missing italic resource.
This compares variable-font metadata, not every glyph byte or the entire file.
Keep all faces and attribution files together when updating a family.

DirectWrite may synthesize the same PostScript name for separate variable
designs. Windows retains each preferred design's original `name` table identity
and portable names separately from its native lookup name. Its picker uses the
font's `fvar` presets and axes; installed static copies do not hide a bundled
variable design of the same family and slant.
