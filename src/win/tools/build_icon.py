"""Package the existing rendered app icon sizes into a Windows ICO container.

Pixels are copied unchanged from assets/icon/Viem.iconset; no imaging library
or platform tools are needed. Re-run after updating those source assets.
"""
from pathlib import Path
import struct

root = Path(__file__).resolve().parents[3]
sources = root / "assets/icon/Viem.iconset"
images = [(16, "icon_16x16.png"), (32, "icon_32x32.png"),
          (64, "icon_32x32@2x.png"), (128, "icon_128x128.png"),
          (256, "icon_256x256.png")]
offset = 6 + 16 * len(images)
entries, data = [], []
for size, name in images:
    png = (sources / name).read_bytes()
    entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                               1, 32, len(png), offset))
    data.append(png)
    offset += len(png)
target = root / "src/win/Assets/Viem.ico"
target.parent.mkdir(parents=True, exist_ok=True)
target.write_bytes(struct.pack("<HHH", 0, 1, len(images)) + b"".join(entries + data))
print(target)
