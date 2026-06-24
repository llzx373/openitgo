#!/usr/bin/env python3
"""Generate macOS app icon PNGs and .icns from source.jpg.

The source image is expected to be a square "macOS icon" style image with a
white background outside the rounded icon shape. This script masks out the
corners using a macOS-style rounded rectangle (≈22% corner radius) so the
resulting icon has transparent corners and displays correctly in the Dock /
Finder.
"""

from pathlib import Path
from PIL import Image, ImageDraw
import shutil
import subprocess

SRC = Path(__file__).parent / "source.jpg"
DST_DIR = Path(__file__).parent

# macOS icon sizes and their @2x counterparts.
# An .iconset needs every size as a separate file.
ICONSET_SIZES = [
    (16, False),
    (16, True),
    (32, False),
    (32, True),
    (128, False),
    (128, True),
    (256, False),
    (256, True),
    (512, False),
    (512, True),
]

# Also keep the legacy individual PNG files for convenience.
LEGACY_SIZES = [16, 32, 64, 128, 256, 512, 1024]


def make_macos_mask(size: int, corner_ratio: float = 0.22) -> Image.Image:
    """Return a grayscale mask: white inside the rounded square, black outside."""
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    radius = int(size * corner_ratio)
    draw.rounded_rectangle((0, 0, size, size), radius=radius, fill=255)
    return mask


def apply_macos_mask(src_image: Image.Image, size: int) -> Image.Image:
    """Resize and mask the source image to a macOS app icon shape."""
    resized = src_image.resize((size, size), Image.Resampling.LANCZOS)
    mask = make_macos_mask(size)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(resized, (0, 0), mask)
    return out


def main() -> None:
    if not SRC.exists():
        raise FileNotFoundError(f"Source icon not found: {SRC}")

    src = Image.open(SRC).convert("RGBA")

    # Generate legacy individual PNGs.
    for size in LEGACY_SIZES:
        icon = apply_macos_mask(src, size)
        icon.save(DST_DIR / f"{size}x{size}.png", "PNG")
        print(f"Generated {size}x{size}.png")

    # Build an .iconset and convert it to .icns.
    iconset_dir = DST_DIR / "AppIcon.iconset"
    if iconset_dir.exists():
        shutil.rmtree(iconset_dir)
    iconset_dir.mkdir()

    for base_size, is_2x in ICONSET_SIZES:
        size = base_size * 2 if is_2x else base_size
        icon = apply_macos_mask(src, size)
        suffix = "@2x" if is_2x else ""
        filename = f"icon_{base_size}x{base_size}{suffix}.png"
        icon.save(iconset_dir / filename, "PNG")
        print(f"Generated iconset {filename}")

    icns_path = DST_DIR / "AppIcon.icns"
    subprocess.run(
        ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(icns_path)],
        check=True,
    )
    shutil.rmtree(iconset_dir)
    print(f"Generated {icns_path}")


if __name__ == "__main__":
    main()
