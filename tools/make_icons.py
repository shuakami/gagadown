#!/usr/bin/env python3
"""Crops the source artwork to a transparent rounded square and writes every icon size."""
import os
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
A = os.path.join(ROOT, "assets")
src = Image.open(os.path.join(A, "icon-source.png")).convert("RGBA")
W = src.width
# Measured corner radius of the artwork is ~242px at 1254px; inset a little so no dark fringe survives.
inset = 8
radius = (W - 2 * inset) // 2
ss = 4
mask = Image.new("L", (W * ss, W * ss), 0)
ImageDraw.Draw(mask).rounded_rectangle((inset * ss, inset * ss, (W - inset) * ss - 1, (W - inset) * ss - 1), radius=radius * ss, fill=255)
mask = mask.resize((W, W), Image.LANCZOS)
out = src.copy()
out.putalpha(mask)
out = out.crop((inset, inset, W - inset, W - inset))
# Recolour any near-black pixels left at the anti-aliased edge with the background blue.
px = out.load()
blue = (44, 146, 253)
for y in range(out.height):
    for x in range(out.width):
        r, g, b, a = px[x, y]
        if 0 < a < 255 and r + g + b < 360:
            px[x, y] = (*blue, a)
big = out.resize((1024, 1024), Image.LANCZOS)
big.save(os.path.join(A, "icon.png"))
big.save(os.path.join(A, "icon.ico"), sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)])
r256 = out.resize((256, 256), Image.LANCZOS)
open(os.path.join(A, "icon_256.rgba"), "wb").write(r256.tobytes())
for s in (16, 32, 48, 128):
    out.resize((s, s), Image.LANCZOS).save(os.path.join(ROOT, "extension", "icons", f"{s}.png"))
print("ok")
