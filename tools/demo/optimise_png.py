#!/usr/bin/env python3
"""Shrink documentation screenshots (used by tools/demo/shots.sh when oxipng isn't installed).

Each PNG is re-saved losslessly with Pillow's optimiser; one still over the size budget is
reduced to a 256-colour palette (with dithering), which keeps screenshots looking the same at
a fraction of the size.

    python3 tools/demo/optimise_png.py docs/images/web/*.png
"""
import io
import sys

from PIL import Image

BUDGET = 400 * 1024


def optimise(path):
    img = Image.open(path)
    img.load()
    out = io.BytesIO()
    img.save(out, "PNG", optimize=True)
    if out.tell() > BUDGET:
        out = io.BytesIO()
        img.convert("RGB").quantize(256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.FLOYDSTEINBERG).save(
            out, "PNG", optimize=True)
    with open(path, "rb") as f:
        before = len(f.read())
    if out.tell() < before:
        with open(path, "wb") as f:
            f.write(out.getvalue())
    print(f"{path}: {before // 1024} KiB -> {min(before, out.tell()) // 1024} KiB")


if __name__ == "__main__":
    for p in sys.argv[1:]:
        optimise(p)
