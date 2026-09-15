# Viem icon

Approved design: an ivory folded-paper V above a wide, thick open-box/space
symbol, with sage paper folds at both corners, on a forest-green rounded square.

## Files

- `Viem.icns`: conventional macOS application icon included in the app bundle.
- `Viem.png`: 1024 × 1024 sRGB PNG with actual alpha transparency outside the green tile.
- `Viem.iconset/`: the ten standard 1× and 2× PNG representations, from 16 to 1024 pixels.
- `Viem-design.png`: the untouched, approved 1254 × 1254 generated preview, including its light exterior background.

`scripts/build-mac-app.sh` copies `Viem.icns` into the app bundle's
`Contents/Resources` before signing. The app's Info.plist selects it through
`CFBundleIconFile`. Both debug and release builds use this asset.
Packaging refreshes the app bundle directory's modification time after signing
so Launch Services can detect icon changes when the app is launched again.

## Export

The original artwork was created and refined with the built-in `image_gen`
tool. The final refinement widened the open-box symbol to approximately 85%
of the V's width, doubled its stroke thickness, and added diagonal paper
folds, sage underside facets, and contact shadows at both corners. The V,
green background, and overall style were retained.

For the icon export, the neutral exterior was removed along the green tile's
existing silhouette while preserving the interior artwork. Core Graphics
exported an sRGB image with alpha, centering the tile within an 824 × 824
area of a 1024 × 1024 canvas, and generated the iconset sizes. Apple's
`iconutil` packaged the iconset as `Viem.icns`.

To repackage the existing iconset from this directory:

```sh
iconutil --convert icns --output Viem.icns Viem.iconset
```

## macOS format distinction

For this conventional `.icns` asset, the area outside the green tile is
transparent, and the green tile itself is opaque. Its outline preserves the
approved design; it is not a mathematically exact Apple canvas mask.

Apple's newer Icon Composer workflow uses a different input: a 1024 × 1024
canvas with an opaque, full-bleed background, plus optional foreground layers.
The system supplies the final corner mask and appearance effects. This folder
does not contain an Icon Composer `.icon` document; importing the already
inset PNG as a full-bleed background would add unwanted padding.

References:

- [Apple app icon guidelines](https://developer.apple.com/design/human-interface-guidelines/app-icons)
- [Preparing artwork for Icon Composer](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer)
- [Apple iconset filenames](https://developer.apple.com/library/archive/documentation/Xcode/Reference/xcode_ref-Asset_Catalog_Format/IconSetType.html)
