#!/usr/bin/env python3
from pathlib import Path
from PIL import Image

src = Path(__file__).parent / "source.jpg"
dst_dir = Path(__file__).parent
sizes = [16, 32, 64, 128, 256, 512, 1024]

img = Image.open(src).convert("RGBA")
for size in sizes:
    resized = img.resize((size, size), Image.Resampling.LANCZOS)
    resized.save(dst_dir / f"{size}x{size}.png", "PNG")

print("Generated PNG icons:", sizes)
