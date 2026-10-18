"""Generates the SILVERMERE login-logo candidates (T-118 phase 1). Run from the repository root:

    python3 -I tools/art/make_logos.py

For each entry in CANDIDATES it lays out SILVERMERE in a TheDraw font from `fonts/`
(see fonts/README.md for where each came from), recolours it into one of SCHEMES, centres
it in 79 columns, adds the optional tagline or frame, and writes
`candidates/NN-font-scheme.ans` plus a PNG next to it (rendered by ansi2png.py). Every
file is checked against the MegaMud-safe rules in `check()` before it is written.

Recolouring keeps each font's own shading but swaps its hues: the distinct foreground
colours a logo uses are sorted by brightness and spread over the scheme's foreground ramp,
and the same for its non-black backgrounds over the scheme's background ramp."""
import os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = os.path.join(HERE, "fonts")
sys.path.insert(0, HERE)
import tdf, ansi2png  # noqa: E402

WIDTH = 79
TEXT = "SILVERMERE"
TAGLINE = "MajorMUD 1.11p"

# Colour ramps, darkest first; ANSI numbers (0-7, +8 bright). Backgrounds must be 0-7.
SCHEMES = {
    "silver": dict(fg=[8, 7, 15], bg=[0, 7], line=8, tag=7),
    "steel": dict(fg=[4, 12, 7, 15], bg=[4, 7], line=4, tag=12),
    "mere": dict(fg=[8, 6, 14, 15], bg=[6, 7], line=6, tag=14),
}

# (font file, font index in the file, scheme, options). "native" keeps the font's own
# colours (already silver or steel); "kern" overrides the font's letter spacing.
CANDIDATES = [
    ("underwld.tdf", 0, "silver", {"tagline": True}),
    ("underwld.tdf", 0, "mere", {"tagline": True}),
    ("metalix.tdf", 0, "silver", {"tagline": True, "native": True}),
    ("falconsx.tdf", 0, "steel", {"tagline": True}),
    ("falconsx.tdf", 0, "mere", {"tagline": True}),
    ("mechanic.tdf", 0, "steel", {"tagline": True, "native": True}),
    ("nyndee.tdf", 0, "silver", {"tagline": True}),
    ("synergy.tdf", 0, "steel", {"frame": True, "kern": 0}),
]


def lum(c):
    r, g, b = ansi2png.PALETTE[c]
    return 0.299 * r + 0.587 * g + 0.114 * b


def spread(colours, ramp):
    colours = sorted(colours, key=lambda c: (lum(c), c))
    n = len(colours)
    return {c: ramp[min(len(ramp) - 1, i * len(ramp) // n)] if n > 1 else ramp[-1]
            for i, c in enumerate(colours)}


def recolour(grid, scheme):
    s = SCHEMES[scheme]
    fgs = {fg for row in grid for ch, fg, bg in row if ch not in (0x20, 0xFF)}
    bgs = {bg for row in grid for _, _, bg in row if bg}
    fmap, bmap = spread(fgs, s["fg"]), spread(bgs, s["bg"])
    bmap[0] = 0
    return [[(ch, fmap.get(fg, 7), bmap[bg]) for ch, fg, bg in row] for row in grid]


def text_row(text, fg):
    return [(b, fg, 0) for b in text.encode("latin-1")]  # each char is a CP437 byte value


def blank(n):
    return [(0x20, 7, 0)] * n


def trim(grid):
    """Drops columns that are blank in every row, at both edges."""
    def empty(x):
        return all(row[x][0] in (0x20, 0xFF) and row[x][2] == 0 for row in grid)
    left, right = 0, len(grid[0])
    while left < right and empty(left):
        left += 1
    while right > left and empty(right - 1):
        right -= 1
    return [row[left:right] for row in grid]


def compose(font_file, index, scheme, opts):
    font = tdf.load(os.path.join(FONTS, font_file))[index]
    grid = font.render(TEXT, kern=opts.get("kern"))
    if not opts.get("native"):
        grid = recolour(grid, scheme)
    grid = trim(grid)
    s = SCHEMES[scheme]
    w = len(grid[0])
    rows = []
    if opts.get("frame"):
        inner = w + 4
        rows.append(text_row("\xda" + "\xc4" * inner + "\xbf", s["line"]))  # top: ┌─┐
        for g in grid:
            rows.append(text_row("\xb3", s["line"]) + blank(2) + g + blank(2) + text_row("\xb3", s["line"]))
        tag = f" {TAGLINE} "
        left = (inner - len(tag)) // 2
        rows.append(text_row("\xc0" + "\xc4" * left, s["line"]) + text_row(tag, s["tag"])
                    + text_row("\xc4" * (inner - left - len(tag)) + "\xd9", s["line"]))
    else:
        rows.extend(grid)
        if opts.get("tagline"):
            tag = f" {TAGLINE} "
            dash = (w - len(tag)) // 2
            rows.append(text_row("\xc4" * dash, s["line"]) + text_row(tag, s["tag"])
                        + text_row("\xc4" * (w - dash - len(tag)), s["line"]))
    width = max(len(r) for r in rows)
    pad = (WIDTH - width) // 2
    return [blank(pad) + r for r in rows]


def encode(rows):
    """Plain SGR, CRLF, each line reset at its end; trailing black spaces dropped."""
    out = bytearray()
    for row in rows:
        while row and row[-1][0] == 0x20 and row[-1][2] == 0:
            row = row[:-1]
        cur = None
        for ch, fg, bg in row:
            if ch == 0x20 and bg == 0 and cur is not None and cur[1] == 0:
                out.append(ch)  # a black space looks the same in any foreground
                continue
            if (fg, bg) != cur:
                codes = ["0"] + (["1"] if fg > 7 else []) + [str(30 + (fg & 7))]
                if bg:
                    codes.append(str(40 + bg))
                out += b"\x1b[" + ";".join(codes).encode() + b"m"
                cur = (fg, bg)
            out.append(ch)
        out += b"\x1b[0m\r\n"
    return bytes(out)


SGR = re.compile(rb"\x1b\[([0-9;]*)m")


def check(data):
    """The MegaMud-safe rules from the T-118 card; returns a list of problems."""
    problems = []
    if not data.endswith(b"\x1b[0m\r\n") and not data.endswith(b"\x1b[0m"):
        problems.append("does not end with ESC[0m")
    if b"\x1a" in data:
        problems.append("contains a SAUCE/EOF marker")
    lines = data.split(b"\r\n")
    if lines and lines[-1] == b"":
        lines.pop()
    if len(lines) > 20:
        problems.append(f"{len(lines)} lines (max 20)")
    for n, line in enumerate(lines, 1):
        if b"\n" in line or b"\r" in line:
            problems.append(f"line {n}: bare CR or LF")
        visible = 0
        i = 0
        while i < len(line):
            if line[i] == 0x1B:
                m = SGR.match(line, i)
                if not m:
                    problems.append(f"line {n}: escape other than SGR")
                    break
                for p in (m.group(1).split(b";") if m.group(1) else [b"0"]):
                    v = int(p or b"0")
                    if not (v in (0, 1, 5) or 30 <= v <= 37 or 40 <= v <= 47):
                        problems.append(f"line {n}: SGR {v}")
                i = m.end()
                continue
            if line[i] < 0x20 or line[i] == 0x7F:
                problems.append(f"line {n}: control byte {line[i]:#04x}")
            visible += 1
            i += 1
        if visible > 79:
            problems.append(f"line {n}: {visible} columns (max 79)")
    return problems


def main():
    out = os.path.join(HERE, "candidates")
    os.makedirs(out, exist_ok=True)
    for n, (font_file, index, scheme, opts) in enumerate(CANDIDATES, 1):
        data = encode(compose(font_file, index, scheme, opts))
        problems = check(data)
        if problems:
            sys.exit(f"candidate {n}: " + "; ".join(problems))
        stem = f"{n:02d}-{font_file[:-4]}-{scheme}"
        with open(os.path.join(out, stem + ".ans"), "wb") as f:
            f.write(data)
        ansi2png.render(ansi2png.parse(data)).save(os.path.join(out, stem + ".png"))
        lines = data.count(b"\r\n")
        print(f"{stem}: {lines} lines, {len(data)} bytes")


if __name__ == "__main__":
    main()
