#!/usr/bin/env python3
"""Oracle for `new_calc_exp_needed`, the experience a level needs.

Plain Python integers, with every unsigned long operation masked to 32 bits as the original
game's 16-bit compiler did it, and the result's billions cut to the low 16 bits and
sign-extended. It is written independently of the Rust version, so the two can be checked
against each other.

    python3 -I tools/parsegen/expneeded.py \
        --out crates/silvermere-rules/tests/fixtures/expneeded.json

Standard library only; run once, the output is committed as a fixture.
"""

import argparse
import json

U32 = 0xFFFFFFFF
BILLION = 1000000000
NEXPSTEPS = 26
EXPEND15 = 54
EXPEND9 = 57
EXPFACT15 = 115
EXPFACT9 = 109
EXPFACT8 = 108
EXPFACTDEN = 100

# The growth table (numerator, denominator) per level step; rows 26-31 are never read.
GROWTH = [
    (1, 1), (40, 20), (44, 24), (44, 24), (48, 28), (48, 28), (52, 32), (52, 32),
    (56, 36), (56, 36), (60, 40), (60, 40), (65, 45), (65, 45), (70, 50), (70, 50),
    (75, 55), (50, 40), (50, 40), (50, 40), (50, 40), (50, 40), (50, 40), (50, 40),
    (50, 40), (23, 20), (23, 20), (23, 20), (23, 20), (23, 20), (23, 20), (23, 20),
]


def new_calc_exp_needed(level, modifier):
    """(billions, remainder) of the experience `level` needs at `modifier`."""
    # (modifier + 100) * 1000 / 100 in signed 32-bit, division truncating toward zero
    v = (modifier + 100) * 1000
    v = abs(v) // 100 * (1 if v >= 0 else -1)
    rem = v & U32
    nbil = 0
    for i in range(level):
        if i < NEXPSTEPS:
            numer, denom = GROWTH[i]
        else:
            if i < EXPEND15:
                numer = EXPFACT15
            elif i < EXPEND9:
                numer = EXPFACT9
            else:
                numer = EXPFACT8
            denom = EXPFACTDEN
        product = (rem * numer) & U32
        if product >= rem and product // numer == rem:  # no overflow
            rem = product // denom
        else:
            scales = 0  # overflow: scale down by 100 until it fits
            while product < rem or product // numer != rem:
                scales += 1
                rem //= 100
                product = (rem * numer) & U32
            if scales <= 1:
                rem = product // denom
            else:
                rem = (((rem * numer) & U32) * 100 & U32) // denom
            while scales > 0:
                rem = (rem * 100) & U32
                scales -= 1
        carry = (((nbil * numer) & U32) * 1000000) & U32  
        while carry >= BILLION:
            carry -= BILLION
            nbil = (nbil + 1) & U32
        total = (rem + carry) & U32
        while total >= BILLION:
            total -= BILLION
            nbil = (nbil + 1) & U32
        rem = total
    bil16 = nbil & 0xFFFF  # cut to 16 bits, then sign-extended back
    if bil16 >= 0x8000:
        bil16 -= 0x10000
    return bil16 & U32, rem


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    rows = []
    # The card's grid: levels 1..60 for modifiers 0, 30, 65 and 100; then every level
    # to 100 for the stock extremes (-20, Human Thief; 200) and Kang Witchunter (65).
    grid = [(m, range(1, 61)) for m in (0, 30, 100)]
    grid += [(m, range(0, 101)) for m in (-20, 65, 200)]
    for modifier, levels in grid:
        for level in levels:
            bil, rem = new_calc_exp_needed(level, modifier)
            rows.append([level, modifier, bil, rem])
    doc = {
        "generated_by": "tools/parsegen/expneeded.py",
        "columns": ["level", "modifier", "expbil", "exprem"],
        "rows": rows,
    }
    with open(args.out, "w", encoding="ascii") as f:
        f.write("{\n")
        for k in ("generated_by", "columns"):
            f.write(f" {json.dumps(k)}: {json.dumps(doc[k])},\n")
        f.write(' "rows": [\n')
        f.write(",\n".join("  " + json.dumps(r) for r in rows))
        f.write("\n ]\n}\n")


if __name__ == "__main__":
    main()
