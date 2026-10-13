# Regenerates assets/logo.txt and assets/logo.ansi (truecolor) from the PhosphorWolf palette.
# Run from the project folder: python3 assets/make-logo.py
import json, sys
LEFT = [0x01, 0x02, 0x04, 0x40]

def eq_cells(bars, peaks, rows=3):
    """One thin bar per braille cell (left dot column), with an optional floating peak dot."""
    total = rows * 4
    out = []
    for r in range(rows):
        line = ""
        for h, p in zip(bars, peaks):
            bits = 0
            for d in range(4):
                from_bottom = total - 1 - (r * 4 + d)
                if from_bottom < h or (p and from_bottom == p - 1):
                    bits |= LEFT[d]
            line += chr(0x2800 + bits)
        out.append(line)
    return out

PLAY = ["⣷⣄ ", "⣿⣿⡷", "⡿⠋ "]
WORD = ["▄▀▀▄ █▄ ▄█ ▄▀▀▄ █▀▀▀ ▄▀▀▀ ▀▀█▀▀ █▀▀▄ ▄▀▀▄",
        "█  █ █ ▀ █ █▀▀█ █▀▀   ▀▀▄   █   █▀█  █  █",
        " ▀▀  ▀   ▀ ▀  ▀ ▀▀▀▀ ▀▀▀    ▀   ▀  ▀  ▀▀ "]
BARS  = [5, 9, 12, 7, 10, 4]
PEAKS = [7, 11, 0, 9, 12, 6]     # 0 = no peak dot (bar already at the top)
EQ = eq_cells(BARS, PEAKS)

X0, Y0 = 1, 1
parts = [("play", X0, PLAY), ("word", X0 + 4, WORD), ("eq", X0 + 4 + len(WORD[0]) + 1, EQ)]
right_edge = parts[-1][1] + len(EQ[0]) - 1

def hexrgb(h): return tuple(int(h[i:i+2], 16) for i in (1, 3, 5))
def mix(a, b, t): return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))
def tohex(c): return "#%02x%02x%02x" % c

# PhosphorWolf colours. Left to right: accent → bright_green → bright_cyan → cyan.
STOPS = [(0.0, "#64ce8d"), (0.45, "#a1dfb4"), (0.75, "#a0d2d5"), (1.0, "#76bac0")]
TOP, BOTTOM = hexrgb("#c1e8cb"), hexrgb("#609174")   # bright_foreground glow on top, dark_foreground underneath
VERTICAL = {0: (TOP, 0.25), 1: (None, 0.0), 2: (BOTTOM, 0.30)}

def colour(x, row):
    t = (x - X0) / (right_edge - X0)
    for (t0, c0), (t1, c1) in zip(STOPS, STOPS[1:]):
        if t <= t1:
            c = mix(hexrgb(c0), hexrgb(c1), (t - t0) / (t1 - t0))
            break
    target, amount = VERTICAL[row]
    return tohex(mix(c, target, amount) if target else c)

objects, ansi_rows = [], ["", "", ""]
for name, x, rows in parts:
    cells = []
    for dy, line in enumerate(rows):
        for dx, ch in enumerate(line):
            if ch not in (" ", "⠀"):
                cells.append([dx, dy, ch, colour(x + dx, dy), None])
    objects.append({"type": "paint", "name": f"logo-{name}", "x": x, "y": Y0, "cells": cells})

# Truecolor ANSI version for terminals, plus plain text.
grid = {}
for o in objects:
    for dx, dy, ch, fg, _ in o["cells"]:
        grid[(o["x"] + dx, dy)] = (ch, fg)
width = right_edge - X0 + 1
plain, ansi = [], []
for dy in range(3):
    p, a = "", ""
    for x in range(X0, right_edge + 1):
        ch, fg = grid.get((x, dy), (" ", None))
        p += ch
        a += f"\x1b[38;2;{int(fg[1:3],16)};{int(fg[3:5],16)};{int(fg[5:7],16)}m{ch}" if fg else " "
    plain.append(p.rstrip())
    ansi.append(a + "\x1b[0m")

json.dump({"type": "group", "name": "Logo", "children": objects}, open("assets/logo-otis.json", "w"), ensure_ascii=False)
open("assets/logo.txt", "w").write("\n".join(plain) + "\n")
open("assets/logo.ansi", "w").write("\n".join(ansi) + "\n")
print("\n".join(plain))
print("colours, left/middle/right on row 2:", colour(X0, 1), colour((X0 + right_edge) // 2, 1), colour(right_edge, 1))
print("row 1 / row 3 at left:", colour(X0, 0), colour(X0, 2))
