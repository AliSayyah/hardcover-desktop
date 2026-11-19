#!/usr/bin/env python3
"""Draws the demo library's covers from catalogue.json with ImageMagick.

Run from anywhere: python3 -I make_covers.py
Writes covers/<book id>.jpg next to this script. Needs `magick` and the Noto
Serif and Noto Sans fonts. The covers are committed, so this only needs to run
again when the catalogue changes.
"""

import json
import os
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "covers")
W, H = 320, 480
FONTS = "/usr/share/fonts/noto"
TITLE_FONT = os.path.join(FONTS, "NotoSerif-Bold.ttf")
AUTHOR_FONT = os.path.join(FONTS, "NotoSans-Medium.ttf")


def caption(path, text, font, size, colour, width, gravity="center", spacing=0):
    subprocess.run(
        [
            "magick", "-background", "none", "-fill", colour, "-font", font,
            "-pointsize", str(size), "-kerning", str(spacing), "-interline-spacing", "-4",
            "-size", f"{width}x", "-gravity", gravity, f"caption:{text}", path,
        ],
        check=True,
    )


def cover(book, tmp):
    c = book["cover"]
    bg, fg, accent = c["bg"], c["fg"], c["accent"]
    title = os.path.join(tmp, "title.png")
    author = os.path.join(tmp, "author.png")
    style = c["style"]
    left = style == "stripes"
    caption(title, book["title"], TITLE_FONT, 30, fg, W - 64, "west" if left else "center")
    caption(author, book["author"].upper(), AUTHOR_FONT, 13, fg, W - 64,
            "west" if left else "center", spacing=2)

    draw = []
    if style == "circle":
        draw = ["-fill", accent, "-draw", f"circle {W // 2},150 {W // 2 + 78},150"]
        title_at, author_at = ("north", "+0+262"), ("south", "+0+34")
    elif style == "split":
        draw = ["-fill", accent, "-draw", f"rectangle 0,{int(H * 0.6)} {W},{H}",
                "-fill", fg, "-draw", f"rectangle 32,{int(H * 0.6) - 3} 96,{int(H * 0.6) + 1}"]
        title_at, author_at = ("north", f"+0+{int(H * 0.6) + 26}"), ("south", "+0+28")
    elif style == "frame":
        draw = ["-fill", "none", "-stroke", accent, "-strokewidth", "3",
                "-draw", f"rectangle 18,18 {W - 18},{H - 18}",
                "-stroke", "none", "-fill", accent,
                "-draw", f"rectangle {W // 2 - 24},{H // 2 + 40} {W // 2 + 24},{H // 2 + 43}"]
        title_at, author_at = ("center", "+0-40"), ("south", "+0+48")
    elif style == "stripes":
        for i, y in enumerate((44, 64, 84)):
            draw += ["-fill", accent, "-draw", f"rectangle 32,{y} {W - 32 - i * 60},{y + 8}"]
        title_at, author_at = ("west", "+32-10"), ("southwest", "+32+40")
    else:
        raise ValueError(f"unknown cover style {style!r}")

    path = os.path.join(OUT, f"{book['id']}.jpg")
    subprocess.run(
        ["magick", "-size", f"{W}x{H}", f"xc:{bg}", *draw,
         title, "-gravity", title_at[0], "-geometry", title_at[1], "-composite",
         author, "-gravity", author_at[0], "-geometry", author_at[1], "-composite",
         "-strip", "-quality", "82", path],
        check=True,
    )


def main():
    with open(os.path.join(HERE, "catalogue.json")) as f:
        catalogue = json.load(f)
    os.makedirs(OUT, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        for book in catalogue["books"]:
            cover(book, tmp)
    print(f"wrote {len(catalogue['books'])} covers to {OUT}")


if __name__ == "__main__":
    main()
