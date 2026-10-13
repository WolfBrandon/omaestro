# OMAESTRO logo 3: OMAESTRO in the Layer 1 letters, the first O wearing braille headphones, then
# a block-bar EQ. The letters are shaded by row except the first O, which is one bright colour;
# the EQ and headphones are one muted green.
# Writes logo-3.txt, logo-3.ansi (truecolor) and logo-3-otis.json (an OTIS group) into the folder given.
#   python3 assets/make-logo-3.py assets
import json, math, os, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "."
X0, Y0 = 1, 0                  # top-left on the OTIS canvas (the headphones' row)
# Bar heights in eighths of a row, measured up from the letters' baseline (halfway down their
# last row). 20 = the letters' full height (2½ rows); every bar needs at least 4.
BARS = [10, 17, 20, 7, 13]
LETTER_ROWS = {1: "#a1dfb4", 2: "#83c99b", 3: "#609174"}   # top, middle, bottom row of OMAESTRO
FIRST_O = "#c1e8cb"                                       # the O under the headphones
OTHER = "#609174"                                         # EQ and headphones

# The Layer 1 letters (OTIS TextArt "blocks", size s), exactly.
WORD = ["▄▀▀▄ █▄ ▄█ ▄▀▀▄ █▀▀▀ ▄▀▀▀ ▀▀█▀▀ █▀▀▄ ▄▀▀▄",
        "█  █ █ ▀ █ █▀▀█ █▀▀   ▀▀▄   █   █▀█  █  █",
        " ▀▀  ▀   ▀ ▀  ▀ ▀▀▀▀ ▀▀▀    ▀   ▀  ▀  ▀▀ "]
O_WIDTH = 4                    # the first O is WORD columns 0..3

DOT = {(0, 0): 0x01, (0, 1): 0x02, (0, 2): 0x04, (0, 3): 0x40,
       (1, 0): 0x08, (1, 1): 0x10, (1, 2): 0x20, (1, 3): 0x80}

# One grid: (col, row) -> char. Row 0 is the headphones' row; letters and bars sit on rows 1–3.
grid, kind = {}, {}
def put(col, row, ch, what):
    assert (col, row) not in grid, f"overlap at {(col, row)}"
    grid[(col, row)], kind[(col, row)] = ch, what
def put_dots(dots, col0, row0, what):
    cells = {}
    for X, Y in dots:
        cells.setdefault((col0 + X // 2, row0 + Y // 4), set()).add((X % 2, Y % 4))
    for k, ds in cells.items():
        put(*k, chr(0x2800 + sum(DOT[d] for d in ds)), what)

# OMAESTRO. Two columns are kept free on each side of the first O for the ear cups.
o_col = 2
for r, line in enumerate(WORD):
    for j, c in enumerate(line):
        if c != " ":
            put(o_col + j + (2 if j >= O_WIDTH else 0), 1 + r, c, "first-o" if j < O_WIDTH else "word")
col = o_col + len(WORD[0]) + 2 + 2           # past the word, plus a 2-column gap

# Headphones in braille over the first O. Dot grid X 0..19 covers columns o_col-3 .. o_col+6
# (the O is X 6..13); Y 0..15 covers rows 0..3. The band only reaches into the lower part of the
# row above the letters.
half = set()
cx, rx, apex, side = 9.5, 6.5, 1, 6
for i in range(301):
    X = 3 + i * 6.5 / 300                       # left half of the arc, X 3 → 9.5
    Y = side - (side - apex) * math.sqrt(max(0.0, 1 - ((X - cx) / rx) ** 2))
    half.add((round(X - 0.01), round(Y)))
    if abs(X - cx) < 3.5:                       # thicker across the top
        half.add((round(X - 0.01), round(Y) + 1))
for X in (2, 3, 4):                             # ear cup with rounded corners, beside the O
    for Y in range(6, 12):
        if not (X == 2 and Y in (6, 11)):
            half.add((X, Y))
dots = half | {(19 - X, Y) for X, Y in half}
put_dots(dots, o_col - 3, 0, "headphones")

# Right: the bar chart in eighth blocks, one column per bar with a gap. The letters end halfway
# down row 3, so each bar ends there too: ▀ in row 3, then blocks growing up from row 2.
EIGHTHS = " ▁▂▃▄▅▆▇█"
for i, h in enumerate(BARS):
    assert 4 <= h <= 20, h
    put(col + i * 2, 3, "▀", "eq")
    for row, start in ((2, 4), (1, 12)):
        level = max(0, min(8, h - start))
        if level:
            put(col + i * 2, row, EIGHTHS[level], "eq")

def colour(k):
    return {"word": LETTER_ROWS.get(k[1]), "first-o": FIRST_O}.get(kind[k], OTHER)
def rgb(h): return tuple(int(h[i:i + 2], 16) for i in (1, 3, 5))

# ---- outputs
children = []
for name, kinds in (("headphones", {"headphones"}), ("word", {"first-o", "word"}), ("eq", {"eq"})):
    keys = sorted((k for k in grid if kind[k] in kinds), key=lambda k: (k[1], k[0]))
    ox = min(k[0] for k in keys)
    children.append({"type": "paint", "name": f"logo3-{name}", "x": X0 + ox, "y": Y0,
                     "cells": [[c - ox, r, grid[(c, r)], colour((c, r)), None] for c, r in keys]})
json.dump({"type": "group", "name": "Logo 3", "children": children},
          open(os.path.join(OUT, "logo-3-otis.json"), "w"), ensure_ascii=False)

width = max(c for c, _ in grid) + 1
height = max(r for _, r in grid) + 1
plain, ansi = [], []
for r in range(height):
    p = a = ""
    for c in range(width):
        ch = grid.get((c, r))
        p += ch or " "
        a += ("\x1b[38;2;%d;%d;%dm" % rgb(colour((c, r))) + ch) if ch else " "
    plain.append(p.rstrip())
    ansi.append(a + "\x1b[0m")
open(os.path.join(OUT, "logo-3.txt"), "w").write("\n".join(plain) + "\n")
open(os.path.join(OUT, "logo-3.ansi"), "w").write("\n".join(ansi) + "\n")
print("\n".join(plain))
print(f"{width}×{height}; parts:", {c["name"]: (c["x"], len(c["cells"])) for c in children})
