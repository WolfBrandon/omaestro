# OMAESTRO logo 2: block-bar EQ · OMAESTRO (3 rows, full height) wearing braille headphones on the
# last O · the same EQ in braille. OMAESTRO is shaded by row (light top, dark bottom); the EQs
# and headphones are one muted green.
# Writes logo-2.txt, logo-2.ansi (truecolor) and logo-2-otis.json (an OTIS group) into the folder given.
#   python3 assets/make-logo-2.py assets
import json, math, os, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "."
X0, Y0 = 1, 11            # top-left on the OTIS canvas (the headphones' row)
BARS = [12, 20, 24, 16, 8]   # bar heights in eighths of a row; 24 = all 3 rows

# 6-pixel-tall letters drawn with half blocks: 3 rows, filling the full height.
FONT = {
    "O": [".###.", "#...#", "#...#", "#...#", "#...#", ".###."],
    "M": ["#...#", "##.##", "#.#.#", "#...#", "#...#", "#...#"],
    "A": [".###.", "#...#", "#...#", "#####", "#...#", "#...#"],
    "E": ["#####", "#....", "####.", "#....", "#....", "#####"],
    "S": [".####", "#....", "#....", ".###.", "....#", "####."],
    "T": ["#####", "..#..", "..#..", "..#..", "..#..", "..#.."],
    "R": ["####.", "#...#", "#...#", "####.", "#..#.", "#...#"],
}
HALF = {("#", "#"): "█", ("#", "."): "▀", (".", "#"): "▄", (".", "."): " "}
def letter(ch):
    bm = FONT[ch]
    return ["".join(HALF[(t, b)] for t, b in zip(top, bot)) for top, bot in zip(bm[0::2], bm[1::2])]

DOT = {(0, 0): 0x01, (0, 1): 0x02, (0, 2): 0x04, (0, 3): 0x40,
       (1, 0): 0x08, (1, 1): 0x10, (1, 2): 0x20, (1, 3): 0x80}

# Everything is placed on one grid: (col, row) -> char, plus braille dot rows for colouring.
# Row 0 is the headphones' row; the letters and bars sit on rows 1–3.
grid, dotrows, letters = {}, {}, set()
def put(col, row, ch):
    assert (col, row) not in grid, f"overlap at {(col, row)}"
    grid[(col, row)] = ch
def put_dots(dots, col0, row0):
    """dots: set of (X, Y) in braille dots, relative to (col0, row0)."""
    cells = {}
    for X, Y in dots:
        k = (col0 + X // 2, row0 + Y // 4)
        cells.setdefault(k, set()).add((X % 2, Y % 4))
    for k, ds in cells.items():
        put(*k, chr(0x2800 + sum(DOT[d] for d in ds)))
        dotrows[k] = [d[1] for d in ds]

# Left: the bar chart in eighth blocks, one column per bar with a gap.
EIGHTHS = " ▁▂▃▄▅▆▇█"
for i, h in enumerate(BARS):
    for r in range(3):
        level = max(0, min(8, h - (2 - r) * 8))
        if level:
            put(i * 2, 1 + r, EIGHTHS[level])
col = len(BARS) * 2 - 1 + 2

# OMAESTRO; the last O gets two extra columns on its left for the ear cup.
for i, ch in enumerate("OMAESTRO"):
    if i == 7:
        col += 2
        o_col = col
    for r, line in enumerate(letter(ch)):
        for j, c in enumerate(line):
            if c != " ":
                put(col + j, 1 + r, c)
                letters.add((col + j, 1 + r))
    col += 6

# Headphones in braille over the last O. Dot grid X 0..21 covers columns o_col-3 .. o_col+7
# (the O itself is X 6..15); Y 0..15 covers rows 0..3. Built for the left half, mirrored.
half = set()
cx, rx, apex, side = 10.5, 7.5, 0, 7
for i in range(301):
    X = 3 + i * 7.5 / 300                       # left half of the arc, X 3 → 10.5
    Y = side - (side - apex) * math.sqrt(max(0.0, 1 - ((X - cx) / rx) ** 2))
    half.add((round(X - 0.01), round(Y)))
    if abs(X - cx) < 4:                         # thicker across the top
        half.add((round(X - 0.01), round(Y) + 1))
for Y in range(side, 8):                        # band side down to the cup
    half.add((3, Y))
for X in (2, 3, 4):                             # ear cup with rounded corners
    for Y in range(8, 14):
        if not (X == 2 and Y in (8, 13)):
            half.add((X, Y))
dots = half | {(21 - X, Y) for X, Y in half}
put_dots(dots, o_col - 3, 0)
col = o_col + 7 + 3

# Right: the same bar chart in braille, two dots wide.
bar_dots = set()
for i, h in enumerate(BARS):
    for k in range(h // 2):                     # 12 dot rows = 3 text rows
        Y = 15 - k
        bar_dots |= {(i * 2, Y), (i * 2 + 1, Y)}
put_dots({(X, Y) for X, Y in bar_dots if Y >= 4}, col, 0)

# ---- colour (PhosphorWolf)
LETTER_ROWS = {1: "#a1dfb4", 2: "#64ce8d", 3: "#19482b"}   # top, middle, bottom row of OMAESTRO
OTHER = "#609174"                                         # EQs and headphones
def rgb(h): return tuple(int(h[i:i + 2], 16) for i in (1, 3, 5))
def styled(col, row):
    return grid[(col, row)], LETTER_ROWS[row] if (col, row) in letters else OTHER, None

# ---- outputs
width = max(c for c, _ in grid) + 1
height = max(r for _, r in grid) + 1
def part(k):
    if k in letters:
        return "word"
    if k in dotrows and k[0] >= o_col - 3 and k[0] <= o_col + 7:
        return "headphones"
    return "eq-left" if k[0] < o_col else "eq-right"
children = []
for name in ("eq-left", "word", "headphones", "eq-right"):
    keys = sorted((k for k in grid if part(k) == name), key=lambda k: (k[1], k[0]))
    if not keys:
        continue
    ox = min(k[0] for k in keys)
    cells = [[c - ox, r, *styled(c, r)] for c, r in keys]
    children.append({"type": "paint", "name": f"logo2-{name}", "x": X0 + ox, "y": Y0, "cells": cells})
assert sum(len(c["cells"]) for c in children) == len(grid), "every cell in exactly one part"
json.dump({"type": "group", "name": "Logo 2 (headphones)", "children": children},
          open(os.path.join(OUT, "logo-2-otis.json"), "w"), ensure_ascii=False)

def sgr(fg, bg):
    s = "\x1b[38;2;%d;%d;%dm" % rgb(fg)
    return s + ("\x1b[48;2;%d;%d;%dm" % rgb(bg) if bg else "\x1b[49m")
plain, ansi = [], []
for r in range(height):
    p = a = ""
    for c in range(width):
        if (c, r) in grid:
            p += grid[(c, r)]
            ch, fg, bg = styled(c, r)
            a += sgr(fg, bg) + ch
        else:
            p += " "
            a += "\x1b[49m "
    plain.append(p.rstrip())
    ansi.append(a + "\x1b[0m")
open(os.path.join(OUT, "logo-2.txt"), "w").write("\n".join(plain) + "\n")
open(os.path.join(OUT, "logo-2.ansi"), "w").write("\n".join(ansi) + "\n")
print("\n".join(plain))
print(f"{width}×{height}; parts:", {c["name"]: len(c["cells"]) for c in children})
