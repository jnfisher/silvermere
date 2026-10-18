"""Renders a CP437 ANSI file to PNG in the IBM VGA 8x16 font, the way ansilove does (T-118).

    python3 -I tools/art/ansi2png.py IN.ans OUT.png [--cols 80]

Understands exactly the MegaMud-safe subset the login art may use: printable CP437
bytes, CR, LF and SGR (`ESC[...m`) with 0, 1, 5, 30-37 and 40-47. Bold makes the
foreground bright; blink is drawn as steady (no iCE colours). Anything else is an error,
so the renderer doubles as a check. The VGA font bitmap is `fonts/vga-8x16.bin`, the
4096-byte PC 80x25 font as shipped in libansilove's `src/fonts/font_pc_80x25.h`. The
ninth pixel column is drawn the VGA way: blank, except box-drawing bytes 0xC0-0xDF repeat
column eight."""
import os, re, sys
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
# The 16 standard VGA palette entries.
PALETTE = [
    (0, 0, 0), (170, 0, 0), (0, 170, 0), (170, 85, 0),
    (0, 0, 170), (170, 0, 170), (0, 170, 170), (170, 170, 170),
    (85, 85, 85), (255, 85, 85), (85, 255, 85), (255, 255, 85),
    (85, 85, 255), (255, 85, 255), (85, 255, 255), (255, 255, 255),
]
SGR = re.compile(rb"\x1b\[([0-9;]*)m")


def parse(data, cols=80):
    """Cells as rows of (byte, fg, bg)."""
    rows, row = [], []
    fg, bg, bold = 7, 0, False
    i = 0
    while i < len(data):
        b = data[i]
        if b == 0x1B:
            m = SGR.match(data, i)
            if not m:
                raise ValueError(f"offset {i}: escape other than SGR")
            for p in (m.group(1).split(b";") if m.group(1) else [b"0"]):
                n = int(p or b"0")
                if n == 0:
                    fg, bg, bold = 7, 0, False
                elif n == 1:
                    bold = True
                elif n == 5:
                    pass
                elif 30 <= n <= 37:
                    fg = n - 30
                elif 40 <= n <= 47:
                    bg = n - 40
                else:
                    raise ValueError(f"offset {i}: SGR {n} is not allowed")
            i = m.end()
            continue
        i += 1
        if b == 0x0D:
            continue
        if b == 0x0A:
            rows.append(row)
            row = []
            continue
        if b < 0x20 or b == 0x7F:
            raise ValueError(f"offset {i - 1}: control byte {b:#04x}")
        row.append((b, fg + (8 if bold else 0), bg))
        if len(row) == cols:
            rows.append(row)
            row = []
    if row:
        rows.append(row)
    return rows


def render(rows, cols=80, font=None):
    if font is None:
        with open(os.path.join(HERE, "fonts", "vga-8x16.bin"), "rb") as f:
            font = f.read()
    img = Image.new("RGB", (cols * 9, max(1, len(rows)) * 16), PALETTE[0])
    px = img.load()
    for y, row in enumerate(rows):
        for x, (ch, fg, bg) in enumerate(row):
            fgc, bgc = PALETTE[fg], PALETTE[bg]
            for gy in range(16):
                bits = font[ch * 16 + gy]
                for gx in range(9):
                    if gx < 8:
                        on = bits & (0x80 >> gx)
                    else:
                        on = 0xC0 <= ch <= 0xDF and bits & 1
                    px[x * 9 + gx, y * 16 + gy] = fgc if on else bgc
    return img


def main(argv):
    cols = 80
    if "--cols" in argv:
        k = argv.index("--cols")
        cols = int(argv[k + 1])
        del argv[k:k + 2]
    src, out = argv
    with open(src, "rb") as f:
        render(parse(f.read(), cols), cols).save(out)


if __name__ == "__main__":
    main(sys.argv[1:])
