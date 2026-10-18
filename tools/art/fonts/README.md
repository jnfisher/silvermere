# Fonts used by the T-118 logo tools

The `.tdf` files are TheDraw fonts from the 1990s ANSI scene, copied byte for byte from
the tdfiglet font collection ("all known TDF fonts"): https://github.com/tat3r/tdfiglet,
`fonts/`, commit `0225b8881d06773b92d9db2c7881f03ff0bdece7` (2018-08-24). TheDraw fonts
carry only a short internal name and no author. Only the fonts the candidates use are kept
here, because `make_logos.py` needs them to regenerate the art. What ships in silvermere is
the generated `.ans` output, not these fonts.

| File | Internal name(s) | Type | sha256 (first 16) |
|---|---|---|---|
| `underwld.tdf` | Underworld | colour | `59b7be49898117bd` |
| `metalix.tdf` | Metalix | colour | `88eefe34f0f766d6` |
| `falconsx.tdf` | Falcons Blue / Gree / Marg / Purp / Red (index 0 used) | colour | `e31d67de04932c16` |
| `mechanic.tdf` | Mechanical | colour | `47f52c774a8514bc` |
| `nyndee.tdf` | Nyndee | colour | `75d9046233cab345` |
| `synergy.tdf` | Synergy | colour | `4e91493974aa33db` |

`vga-8x16.bin` is the IBM PC VGA 8x16 text-mode font (256 glyphs, 16 bytes each), which
`ansi2png.py` uses to draw PNGs. Its bytes come from the `font_pc_80x25` array in
libansilove's `src/fonts/font_pc_80x25.h` (https://github.com/ansilove/libansilove, commit
`e22973d569c97c1a521a5246e057d300e865bd9f`), converted from C hex to raw binary.
