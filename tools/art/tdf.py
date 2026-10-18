"""Reads TheDraw .tdf font files and lays out text in them (T-118).

A small stand-in for tdfiglet: it parses the TheDraw font format and returns a grid of
cells (CP437 byte, foreground 0-15, background 0-7) that `make_logos.py` colours and
writes out as ANSI. Colours come back in ANSI order
(0 black, 1 red, ..., 7 white; +8 bright). Supports block fonts (type 1, characters only) and colour fonts
(type 2, a character and an attribute byte per cell); outline fonts (type 0) are skipped.

File layout: a 0x13 byte, "TheDraw FONTS file", 0x1A; then one or more fonts, each
`55 AA 00 FF`, a name length byte and 12-byte name, 4 reserved bytes, the type, the
letter spacing, a u16 data size, 94 u16 offsets for '!'..'~' (0xFFFF = missing) and the
glyph data. Each glyph is a width byte, a height byte, then cells; 0x0D ends a row and
0x00 ends the glyph. In block fonts '&' marks a transparent cell."""
import struct

MAGIC = b"\x13TheDraw FONTS file\x1a"
FIRST = 0x21  # '!'
NCHARS = 94
# TheDraw attributes use the DOS colour order (1 = blue); cells use ANSI order (1 = red).
DOS_TO_ANSI = [0, 4, 2, 6, 1, 5, 3, 7]


class Font:
    def __init__(self, name, ftype, spacing, offsets, data):
        self.name, self.type, self.spacing = name, ftype, spacing
        self.offsets, self.data = offsets, data
        self._glyphs = {}

    def has(self, ch):
        if ch == " ":
            return True
        i = ord(ch) - FIRST
        return 0 <= i < NCHARS and self.offsets[i] != 0xFFFF and self.offsets[i] < len(self.data)

    def glyph(self, ch):
        """Rows of cells; a cell is (byte, fg, bg) or None for transparent."""
        if ch in self._glyphs:
            return self._glyphs[ch]
        p = self.offsets[ord(ch) - FIRST]
        d = self.data
        width, height = d[p], d[p + 1]
        p += 2
        rows, row = [], []
        while p < len(d) and d[p] != 0:
            b = d[p]
            p += 1
            if b == 0x0D:
                rows.append(row)
                row = []
                continue
            if self.type == 2:
                attr = d[p]
                p += 1
                fg, bg = attr & 0x0F, (attr >> 4) & 0x07
                cell = (b if b >= 0x20 else 0x20, DOS_TO_ANSI[fg & 7] | (fg & 8), DOS_TO_ANSI[bg])
            else:
                cell = None if b == ord("&") else (b if b >= 0x20 else 0x20, 7, 0)
            row.append(cell)
        if row:
            rows.append(row)
        rows = [r + [(0x20, 7, 0)] * (width - len(r)) for r in rows]
        while len(rows) < height:
            rows.append([(0x20, 7, 0)] * width)
        self._glyphs[ch] = rows
        return rows

    def height(self, text):
        return max(len(self.glyph(c)) for c in text if c != " ")

    def render(self, text, space=None, kern=None):
        """Lays `text` out left to right; returns rows of cells. `space` is the width of a
        blank; `kern` overrides the font's own letter spacing."""
        gap = self.spacing if kern is None else kern
        if space is None:
            space = max(3, len(self.glyph("I")[0]) if self.has("I") else 4)
        h = self.height(text)
        out = [[] for _ in range(h)]
        for n, c in enumerate(text):
            if c == " ":
                g = [[(0x20, 7, 0)] * space for _ in range(h)]
            else:
                g = self.glyph(c)
                w = len(g[0])
                g = g + [[(0x20, 7, 0)] * w for _ in range(h - len(g))]
            for y in range(h):
                if n:
                    out[y].extend([(0x20, 7, 0)] * gap)
                out[y].extend(x if x is not None else (0x20, 7, 0) for x in g[y])
        return out


def load(path):
    """Every usable (block or colour) font in the file."""
    with open(path, "rb") as f:
        m = f.read()
    if not m.startswith(MAGIC):
        raise ValueError(f"{path}: not a TheDraw font file")
    fonts, p = [], len(MAGIC)
    while p + 233 - 20 <= len(m) and m[p:p + 4] == b"\x55\xaa\x00\xff":
        n = m[p + 4]
        name = m[p + 5:p + 5 + min(n, 12)].decode("cp437")
        ftype, spacing = m[p + 21], m[p + 22]
        (size,) = struct.unpack_from("<H", m, p + 23)
        offsets = struct.unpack_from("<94H", m, p + 25)
        data = m[p + 213:p + 213 + size]
        if ftype in (1, 2):
            fonts.append(Font(name, ftype, spacing, offsets, data))
        p += 213 + size
    return fonts
