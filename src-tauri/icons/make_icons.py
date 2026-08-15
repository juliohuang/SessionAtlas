"""Generate SessionAtlas app icons from a source PNG into src-tauri/icons/.

Produces the five files referenced by tauri.conf.json:
  32x32.png, 128x128.png, 128x128@2x.png, icon.ico, icon.icns

By default the checked-in ``source.png`` is used. Pass an explicit path as the
first CLI arg to replace the canonical source:
  python src-tauri/icons/make_icons.py path/to/source.png

The source is embedded onto the app's background colour (#0e0d0b) so the
icon has no jaggies/halo on Windows (which ignores alpha for the taskbar)
and a clean square tile in macOS/Linux.
"""
import os
import sys
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SRC = os.path.join(HERE, "source.png")
BG = (14, 13, 11)   # #0e0d0b — window backgroundColor; matches tauri.conf.json


def composite_on_bg(src_path, size):
    """Open `src_path`, composite it onto the solid BG at `size`×`size`."""
    base = Image.open(src_path).convert("RGBA")
    # Fit the logo onto a square tile, leaving a small margin so it isn't
    # flush with the edge at small sizes.
    margin = int(size * 0.10)
    inner = size - margin * 2
    logo = base.resize((inner, inner), Image.LANCZOS)
    tile = Image.new("RGBA", (size, size), BG + (255,))
    tile.alpha_composite(logo, (margin, margin))
    return tile


def main():
    src_path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SRC
    if not os.path.exists(src_path):
        sys.exit(f"source icon not found: {src_path}")

    # The checked-in source is already a 1024px composited tile, which keeps
    # default regeneration idempotent. Explicit raw logos receive the margin
    # and background treatment once before becoming the new canonical source.
    if os.path.abspath(src_path) == os.path.abspath(DEFAULT_SRC):
        src = Image.open(src_path).convert("RGBA").resize((1024, 1024), Image.LANCZOS)
    else:
        src = composite_on_bg(src_path, 1024)

    # PNGs referenced by tauri.conf.json bundle.icons.
    for name, sz in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256)]:
        src.resize((sz, sz), Image.LANCZOS).save(os.path.join(HERE, name))
        print("wrote", name)

    # ICO (multi-size, embedded) — Windows taskbar/start menu.
    ico_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    src.save(os.path.join(HERE, "icon.ico"), format="ICO", sizes=ico_sizes)
    print("wrote icon.ico")

    # ICNS — macOS. PIL needs a >=512 image; 1024 source satisfies that.
    try:
        src.save(os.path.join(HERE, "icon.icns"), format="ICNS")
        print("wrote icon.icns")
    except Exception as e:
        # Non-macOS hosts sometimes can't write a real icns; fall back to a
        # PNG so the file exists (only matters when bundling for macOS).
        src.resize((256, 256), Image.LANCZOS).save(
            os.path.join(HERE, "icon.icns"), format="PNG")
        print("wrote icon.icns (PNG fallback:", e, ")")

    # Keep a copy of the high-res source for future regenerations.
    src.save(os.path.join(HERE, "source.png"))
    print("wrote source.png")


if __name__ == "__main__":
    main()
