"""Generate transparent SessionAtlas icons from the editable source.svg.

Run: python src-tauri/icons/make_icons.py [optional-source.svg-or.png]
Dependencies: Pillow and CairoSVG (for SVG input).
All formats preserve alpha; no opaque background is added.
"""
from io import BytesIO
from pathlib import Path
import sys

from PIL import Image, ImageOps

HERE = Path(__file__).resolve().parent
DEFAULT_SRC = HERE / "source.svg"


def load_source(path):
    if path.suffix.lower() == ".svg":
        import cairosvg

        data = cairosvg.svg2png(url=str(path), output_width=1024, output_height=1024)
        image = Image.open(BytesIO(data)).convert("RGBA")
    else:
        image = Image.open(path).convert("RGBA")
    image = ImageOps.contain(image, (1024, 1024), Image.Resampling.LANCZOS)
    canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    canvas.alpha_composite(image, ((1024 - image.width) // 2, (1024 - image.height) // 2))
    if canvas.getchannel("A").getextrema() != (0, 255):
        raise ValueError("Icon source must contain both transparent and opaque pixels")
    return canvas


def main():
    source = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else DEFAULT_SRC
    src = load_source(source)
    for name, size in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256)]:
        src.resize((size, size), Image.Resampling.LANCZOS).save(HERE / name)
        print("wrote", name)
    sizes = [(size, size) for size in (16, 24, 32, 48, 64, 128, 256)]
    src.save(HERE / "icon.ico", format="ICO", sizes=sizes)
    # Fail visibly if encoding is unavailable; never disguise a PNG as ICNS.
    src.save(HERE / "icon.icns", format="ICNS")
    src.save(HERE / "source.png")
    print("wrote icon.ico, icon.icns, source.png (transparent RGBA)")


if __name__ == "__main__":
    main()
