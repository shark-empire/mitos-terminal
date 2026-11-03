#!/usr/bin/env python3
"""Regenerates AppIcon-1024.png and AppIcon.icns (needs Pillow: `pip3 install pillow`).

You do not need to run this to build the app: both files are committed. Run it only if you
change the artwork below. The colours are the default "Futuristic Sci-Fi" theme's
(src/theme.rs): near-black background, cyan prompt.
"""
import io
import struct
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
SS = 2                       # supersampling factor for smooth edges
SIZE = 1024 * SS

BG_TOP = (15, 23, 38)        # theme.surface_alt
BG_BOTTOM = (5, 9, 17)       # theme.bg
BORDER = (45, 110, 150)      # theme.border
CYAN = (95, 212, 255)        # theme.cursor / glow
FG = (196, 210, 226)         # theme.fg


def s(v):
    return int(round(v * SS))


def rounded_mask(box, radius):
    m = Image.new("L", (SIZE, SIZE), 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius=radius, fill=255)
    return m


def vertical_gradient(top, bottom):
    g = Image.new("RGB", (1, SIZE))
    px = g.load()
    for y in range(SIZE):
        t = y / (SIZE - 1)
        px[0, y] = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
    return g.resize((SIZE, SIZE))


def glyph_layer(color):
    """The `>_` prompt, drawn in `color` on a transparent layer."""
    layer = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    stroke = s(72)
    pts = [(s(318), s(372)), (s(512), s(512)), (s(318), s(652))]
    d.line(pts, fill=color + (255,), width=stroke, joint="curve")
    for (x, y) in (pts[0], pts[2], pts[1]):      # round caps and join
        r = stroke // 2
        d.ellipse((x - r, y - r, x + r, y + r), fill=color + (255,))
    d.rounded_rectangle((s(560), s(624), s(742), s(688)), radius=s(32), fill=FG + (255,))
    return layer


def build():
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))

    box = (s(100), s(100), s(924), s(924))        # macOS icon grid: 824 px tile on a 1024 canvas
    radius = s(185)
    tile = rounded_mask(box, radius)

    # soft drop shadow
    shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    shadow.putalpha(tile.filter(ImageFilter.GaussianBlur(s(26))).point(lambda v: int(v * 0.55)))
    canvas.alpha_composite(shadow.transform(shadow.size, Image.AFFINE, (1, 0, 0, 0, 1, -s(14))))

    # tile: gradient, faint top sheen, thin border
    body = vertical_gradient(BG_TOP, BG_BOTTOM).convert("RGBA")
    sheen = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(sheen).ellipse((s(-200), s(-520), s(1224), s(420)), fill=(255, 255, 255, 14))
    body.alpha_composite(sheen)
    body.putalpha(tile)
    canvas.alpha_composite(body)
    ring = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(ring).rounded_rectangle(box, radius=radius, outline=BORDER + (170,), width=s(4))
    canvas.alpha_composite(ring)

    # glyph with a restrained glow
    glyph = glyph_layer(CYAN)
    glow = glyph.filter(ImageFilter.GaussianBlur(s(34)))
    glow.putalpha(glow.getchannel("A").point(lambda v: int(v * 0.55)))
    canvas.alpha_composite(glow)
    canvas.alpha_composite(glyph)

    return canvas.resize((1024, 1024), Image.LANCZOS)


# ICNS: 'icns' + total length, then (type, length, PNG) entries. PNG payloads are accepted for
# these types by macOS 10.7 and later.
ICNS_TYPES = [
    ("icp4", 16), ("icp5", 32), ("icp6", 64),
    ("ic07", 128), ("ic08", 256), ("ic09", 512), ("ic10", 1024),
    ("ic11", 32), ("ic12", 64), ("ic13", 256), ("ic14", 512),
]


def png_bytes(img, px):
    buf = io.BytesIO()
    img.resize((px, px), Image.LANCZOS).save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def write_icns(master, path):
    cache = {}
    body = b""
    for tag, px in ICNS_TYPES:
        data = cache.setdefault(px, png_bytes(master, px))
        body += tag.encode("ascii") + struct.pack(">I", len(data) + 8) + data
    path.write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + body)


if __name__ == "__main__":
    master = build()
    master.save(HERE / "AppIcon-1024.png", optimize=True)
    write_icns(master, HERE / "AppIcon.icns")
    print("wrote AppIcon-1024.png and AppIcon.icns")
