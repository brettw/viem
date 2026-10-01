# Bundled fonts

These original desktop font files are shared by both native frontends. App
packaging copies this directory to `Resources/fonts`, including the upstream
licenses, copyright notices and documentation. Fonts remain app-local; Viem
does not install them for other applications or embed file paths in saved styles.

- `recursive/recursive-static-TTFs.ttc`: Recursive 1.085 from the supplied
  Arrow Type desktop release, containing 64 static faces in the Mono/Sans and
  Casual/Linear families. The original `LICENSE.txt` and `README.md` accompany
  the collection.
- `flightline/*.ttf`: the 12 supplied Flightline Code faces (six weights with
  upright and italic variants). The original `OFL.txt`, `README.md` and
  `TRADEMARKS.md` retain the distribution's copyright and licensing information.

Font binaries are copied byte-for-byte. macOS registers them with Core Text at
process scope before creating editor UI. Windows indexes the bundled files with
Win2D alongside system fonts, and selects a file URI only when rendering. Keep
all faces and attribution files together when updating a family.
