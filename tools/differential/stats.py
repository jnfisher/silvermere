#!/usr/bin/env python3
"""The M2 statistical and record-level comparisons (T-040; layer 4 in docs/testing.md). Stdlib only, like bridge.py.

The statistics are `crates/silvermere-testkit/src/stats.rs` line for line (same pooling rule,
same incomplete-gamma series and continued fraction), so both sides compute the same statistic
on the same counts; `python3 -I stats.py selftest` checks the numbers the Rust tests pin.

What is compared, for a random scenario run on both servers (events unmasked, from the
report's `orig.raw`/`new.raw` through the pattern registry):

  hit rate          the player's swings that land (you_hit, you_glance, you_dodged) over all
                    swings; two-proportion z, and |difference| <= 2 points; each side needs
                    >= --min-swings swings (2,000)
  monster hit rate  the sheet monster's swings at the player that land: two-proportion z, no
                    bound (John, 2026-10-17, option C)
  outcome mix       hit / miss / dodged / glance: chi-square homogeneity
  damage histogram  the dmg of you_hit: chi-square homogeneity
  blows per round   player swings per round window (a script step that only waits one energy
                    round, `"round": true`): chi-square homogeneity
  regen per tick    the set of hit point gains between two prompts with nothing else between:
                    identical. A gain that ends on maxhp is left out: the game clips there, so it is the remainder, not a regen
                    value (run 39035493410: 33 -> 35 of a +3). maxhp is the `st` line's
                    `Hits: hp/maxhp` (st_hits) after the gain, else the one before it; with
                    none, the highest hit points the run shows, and the row says so
  spawns per minute monsters created in the room ("... into the room from nowhere") over the
                    scenario's wall-clock time: the conditional binomial test of two Poisson
                    rates

Every gated row decided by a test is one of a family: Holm-Bonferroni at family alpha 0.01 over
all of them in the report (`holm`; John, 2026-10-17). The report shows each row's raw p, its
Holm threshold and its adjusted p.

Advisory (reported, flagged at p <= 0.01, never failing; John: arena crowding is not the spawn
check, most play is not in an arena):
  wanderers per minute  monsters walking in from a neighbouring room
  monster noise per minute  monsters moving next door (random_events.json)

and after the script, record level: the original's WCCUSERS row (decoded by the vendored
mmud_data.py) against the new server's `characters` row (hp, maxhp, exp, level, cp, lives,
stats, coins, inventory and worn slots, settings), and WCCACMSR against `active_monsters`
counted per room.

  stats.py selftest
  stats.py report --report DIR [--report DIR2 ...] [--scenarios m2] [--out DIR]
                                                     stats.md/stats.json from harness reports;
                                                     several reports (shards) are pooled
  stats.py records --orig-db DIR --new-db FILE --name NAME

differential.py calls `scenario_checks` for every scenario with an `m2` block and puts the
checks into the scenario's summary.json (`stats`) and its pass/fail; `ab_sheets.json` (from
`$SILVERMERE_AB_EXPORT`, written by `cargo test -p silvermere-testkit --test all stock_ab_sheets::`) adds the
closed-form checks for a scenario that names its `sheet`.
"""
import argparse
import collections
import json
import math
import os
import re
import sqlite3
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

ALPHA = 0.01
MIN_EXPECTED = 5.0
HIT_RATE_BOUND = 0.02
MIN_RATE_SECONDS = 60.0   # a side watched for less is no sample for a rate
MIN_SWINGS = 2000

# ---- the statistics (stats.rs) ------------------------------------------------------------

_LANCZOS = [
    0.9999999999998099, 676.5203681218851, -1259.1392167224028, 771.3234287776531,
    -176.6150291621406, 12.507343278686905, -0.13857109526572012, 9.984369578019572e-6,
    1.5056327351493116e-7,
]


def ln_gamma(x):
    """ln Γ(x) for x > 0 (Lanczos, g = 7, nine coefficients)."""
    if x < 0.5:
        return math.log(math.pi / math.sin(math.pi * x)) - ln_gamma(1.0 - x)
    x -= 1.0
    a = _LANCZOS[0]
    t = x + 7.5
    for i in range(1, 9):
        a += _LANCZOS[i] / (x + i)
    return 0.5 * math.log(2.0 * math.pi) + (x + 0.5) * math.log(t) - t + math.log(a)


def gamma_q(a, x):
    """The regularised upper incomplete gamma Q(a, x) (Numerical Recipes gammq)."""
    if x <= 0.0:
        return 1.0
    gln = ln_gamma(a)
    if x < a + 1.0:
        ap, total = a, 1.0 / a
        delta = total
        for _ in range(1000):
            ap += 1.0
            delta *= x / ap
            total += delta
            if abs(delta) < abs(total) * 1e-15:
                break
        p = total * math.exp(-x + a * math.log(x) - gln)
        return max(1.0 - p, 0.0)
    tiny = 1e-300
    b = x + 1.0 - a
    c = 1.0 / tiny
    d = 1.0 / b
    h = d
    for i in range(1, 1000):
        an = -i * (i - a)
        b += 2.0
        d = an * d + b
        if abs(d) < tiny:
            d = tiny
        c = b + an / c
        if abs(c) < tiny:
            c = tiny
        d = 1.0 / d
        delta = d * c
        h *= delta
        if abs(delta - 1.0) < 1e-15:
            break
    return math.exp(-x + a * math.log(x) - gln) * h


def chi_square_sf(stat, df):
    return 1.0 if df == 0 else gamma_q(df / 2.0, stat / 2.0)


def normal_two_sided(z):
    t = abs(z) / math.sqrt(2.0)
    return gamma_q(0.5, t * t)


def test(statistic, df, p):
    return {"statistic": statistic, "df": df, "p_value": p}


def pool(observed, expected):
    """Adjacent bins merged left to right until each expected count reaches 5; a short last
    run joins the bin before it."""
    out, o, e = [], 0.0, 0.0
    for ob, ex in zip(observed, expected):
        o += ob
        e += ex
        if e >= MIN_EXPECTED:
            out.append([o, e])
            o = e = 0.0
    if o > 0 or e > 0:
        if out:
            out[-1][0] += o
            out[-1][1] += e
        else:
            out.append([o, e])
    return [tuple(b) for b in out]


def chi_square_gof(observed, probs):
    n = sum(observed)
    bins = pool([float(c) for c in observed], [p * n for p in probs])
    stat = sum((o - e) ** 2 / e for o, e in bins if e > 0)
    df = max(len(bins) - 1, 0)
    return test(stat, df, chi_square_sf(stat, df))


def chi_square_homogeneity(a, b):
    na, nb = sum(a), sum(b)
    if na == 0 or nb == 0:
        return test(0.0, 0, 1.0)
    total = na + nb
    small = min(na, nb) / total
    groups, ga, gb, gw = [], 0, 0, 0.0
    for x, y in zip(a, b):
        ga += x
        gb += y
        gw += (x + y) * small
        if gw >= MIN_EXPECTED:
            groups.append([ga, gb])
            ga = gb = 0
            gw = 0.0
    if ga + gb > 0:
        if groups:
            groups[-1][0] += ga
            groups[-1][1] += gb
        else:
            groups.append([ga, gb])
    stat = 0.0
    for x, y in groups:
        c = x + y
        ea, eb = c * na / total, c * nb / total
        if ea > 0:
            stat += (x - ea) ** 2 / ea
        if eb > 0:
            stat += (y - eb) ** 2 / eb
    df = max(len(groups) - 1, 0)
    return test(stat, df, chi_square_sf(stat, df))


def two_proportion_z(x1, n1, x2, n2):
    p = (x1 + x2) / (n1 + n2)
    se = math.sqrt(p * (1 - p) * (1 / n1 + 1 / n2))
    z = (x1 / n1 - x2 / n2) / se if se > 0 else 0.0
    return test(z, 0, normal_two_sided(z))


def binomial_z(x, n, p):
    se = math.sqrt(n * p * (1 - p))
    if se > 0:
        z = (x - n * p) / se
        return test(z, 0, normal_two_sided(z))
    return test(0.0, 0, 1.0 if abs(x - n * p) < 0.5 else 0.0)


def poisson_rates(x1, t1, x2, t2):
    """Are x1 events in t1 and x2 in t2 one rate? Conditional on x1+x2, x1 ~ Bin(n, t1/(t1+t2))."""
    n = x1 + x2
    if n == 0 or t1 <= 0 or t2 <= 0:
        return test(0.0, 0, 1.0)
    return binomial_z(x1, n, t1 / (t1 + t2))


# ---- samples from events -------------------------------------------------------------------

SWING = {"you_hit": "hit", "you_miss": "miss", "you_dodged": "dodged", "you_glance": "glance"}
OUTCOMES = ["hit", "miss", "dodged", "glance"]
# a monster's swing at this player, by its game-data text (world_json.py: the attack slot's
# hit, defence and miss texts, WCCMSG; attack_monster_user): one session counts
# only the swings at itself, so parallel sessions in one room never count a swing twice
MON_SWING = {"mon_hits_you": "hit", "mon_deflect_you": "deflect", "mon_dodge_you": "dodge",
             "mon_miss_you": "miss"}
MON_OUTCOMES = ["hit", "deflect", "dodge", "miss"]
ARRIVAL_EVENTS = {"monster_arrives_generic", "monster_arrives"}
# a monster created in the room arrives "from nowhere"; one that walked in
# arrives from a direction (from the <dir>, from above, from below)
WANDER_RX = re.compile(r" into the (?:room )?from (?:above|below|the \w+)")
# the data-driven arrival lines (WCCMSG arrivemsg) the registry without game
# data leaves unclassified, and the built-in ones
# (any verb: the data's arrival texts vary, "scuttles" among them)
ARRIVAL_RX = re.compile(r"\w+ into the (?:room )?from (?:nowhere|above|below|the )|just arrived from ")


def _of_monster(f, monster):
    """Whether an event's monster (`mon` or `target`, adjectives and all: `large giant rat`)
    is the sheet's `monster`; any when no sheet is named."""
    if not monster:
        return True
    who = str(f.get("base") or f.get("mon") or f.get("target") or "").strip().lower()
    return who == monster or who.endswith(" " + monster)


def samples(events, round_steps=(), monster=None, watch_steps=None):
    """What one side's run gave: swing outcomes, damage, blows per round window, regen gains and
    monster arrivals: `arrivals` the spawns, `wanders` the walk-ins. `events`: [{"step", "name",
    "fields", "line"?}] unmasked. The registry's monster_arrives_generic keeps no line: a `dir`
    field makes it a walk-in, and without one it counts as a spawn (from above or below is a
    walk-in too, but the Newhaven arena's only vertical exit is the group-0 road, which sends
    no monster down)."""
    out = {"swings": collections.Counter(), "damage": collections.Counter(),
           "blows": collections.Counter(), "regen": collections.Counter(), "arrivals": 0,
           "wanders": 0, "noise": 0, "mon_swings": collections.Counter(),
           "mon_damage": collections.Counter()}
    per_step = collections.Counter()
    last_hp, between = None, False
    gains = []   # (gain, hit points after it, event index), the clipped ones dropped below
    top = None   # the highest hit points any prompt shows
    caps = []    # (event index, maxhp) of each st_hits
    for i, e in enumerate(events):
        name, f = e["name"], e.get("fields", {})
        if name == "st_hits" and str(f.get("maxhp", "")).isdigit():
            caps.append((i, int(f["maxhp"])))
        # `monster` (a sheet scenario, m2.monster): only swings at or by that monster count;
        # others in the room (left from earlier runs, or wandering in) are not the sheet
        if name in SWING and _of_monster(f, monster):
            out["swings"][SWING[name]] += 1
            per_step[e.get("step")] += 1
            if name == "you_hit" and str(f.get("dmg", "")).isdigit():
                out["damage"][int(f["dmg"])] += 1
        if name in MON_SWING and _of_monster(f, monster):
            out["mon_swings"][MON_SWING[name]] += 1
            if name == "mon_hits_you" and str(f.get("dmg", "")).isdigit():
                out["mon_damage"][int(f["dmg"])] += 1
        # `watch_steps` (the wander watch): arrivals and noise count only in its watch steps,
        # and only of the sheet's monster, so the watcher's own summons and fights are not
        # counted
        watching = watch_steps is None or (e.get("step") in watch_steps
                                           and _of_monster(f, monster))
        if name == "noise" and watching:
            out["noise"] += 1
        if watching and (name in ARRIVAL_EVENTS
                         or (name == "unknown" and ARRIVAL_RX.search(e.get("line", "")))):
            # the game-data arrival (monster_arrives) names where from: `nowhere` is a spawn
            frm = str(f.get("frm", ""))
            if f.get("dir") or WANDER_RX.search(e.get("line", "")) or (frm and frm != "nowhere"):
                out["wanders"] += 1
            else:
                out["arrivals"] += 1
        if name == "prompt" and re.fullmatch(r"-?\d+", str(f.get("hp", ""))):
            hp = int(f["hp"])
            top = hp if top is None else max(top, hp)
            if last_hp is not None and not between and hp > last_hp:
                gains.append((hp - last_hp, hp, i))
            last_hp, between = hp, False
        elif name not in ("prompt", "echo"):
            between = True
    out["regen_cap_from_top"] = 0
    for gain, hp, i in gains:
        after = [m for j, m in caps if j > i]
        before = [m for j, m in caps if j < i]
        if after or before:
            cap = after[0] if after else before[-1]
        else:
            cap = top
            out["regen_cap_from_top"] += 1
        if hp != cap:
            out["regen"][gain] += 1
    for s in round_steps:
        if per_step.get(s):
            out["blows"][per_step[s]] += 1
    return out


def _vec(counter, keys):
    return [counter.get(k, 0) for k in keys]


MON_ROUNDS_NOTE = ("MBBSEmu artifact (John, 2026-10-17): its 21-slot active-monster cache "
                   "(farcoreleft -1) makes evictions follow a monster numbering silvermere "
                   "cannot reproduce; maxactive reads 27 on the original (the cold-start value; "
                   "why the restart keeps it is under investigation; MMUDACT.LOG captures "
                   "`Max Active:`); advisory")
REGEN_SAMPLE = 3   # gains a side needs before its set of gains says anything


def short_note(a, b, need, required, enough, bound=None):
    """The note of a hit-rate row whose sample is short of `need` (it fails for that alone when
    required), so the report tells a short sample from a mismatch."""
    if enough or not required:
        return {}
    unless = "unless Holm rejects its p" + (
        f" or the difference is above {100 * bound:.0f} points" if bound else "")
    return {"note": f"sample short (orig {a}, new {b}; {need} needed a side), not a mismatch "
                    + unless}


def holm(checks, alpha=ALPHA):
    """Holm-Bonferroni over the gated rows the significance test decides (John, 2026-10-17:
    family alpha 0.01 across all the gated rows of the report). The k-th smallest of the m raw
    p-values is held to alpha / (m - k + 1), stepping down until one is not rejected; each row
    gets its threshold (`holm_alpha`), its adjusted p (`holm_p`) and a pass of its other
    conditions (`held`) and not rejected. Rows outside the family keep their pass. In place;
    returns the family size."""
    family = sorted((c for c in checks if c.get("tested") and not c.get("advisory")),
                    key=lambda c: c["p_value"])
    m = len(family)
    adjusted, rejecting = 0.0, True
    for k, c in enumerate(family):
        c["holm_alpha"] = alpha / (m - k)
        adjusted = max(adjusted, min(1.0, (m - k) * c["p_value"]))
        c["holm_p"] = adjusted
        rejecting = rejecting and c["p_value"] <= c["holm_alpha"]
        c["rejected"] = rejecting
        c["pass"] = c.get("held", True) and not rejecting
        c["holm_m"] = m
    return m


def compare_samples(o, n, durations=(0.0, 0.0), min_swings=MIN_SWINGS, min_regen=0,
                    gate_wanders=False, monster_required=False):
    """[{what, pass, orig, new, statistic, df, p_value, ...}] for one scenario.

    `min_swings` > 0 (the fight sample) makes the sample and the player's 2-point hit-rate
    bound a requirement: fewer swings fail. At 0 (a short scenario) the bound applies from
    MIN_SWINGS a side (`enough`); below it only the test fails the row. The rows' passes here
    use the raw p against 0.01; `holm` decides them over the family. Likewise `min_regen` > 0 requires
    that many hit point gains a side (the rest scenario); at 0 a side with fewer than
    REGEN_SAMPLE gains makes the regen check informational."""
    checks = []

    def add(what, t, ok, **kw):
        checks.append(dict({"what": what, "pass": bool(ok)}, **t, **kw))

    def tested(what, t, held=True, **kw):
        # a row the significance test decides: `held` is the rest of its condition (the bound,
        # the sample), its p joins the report's Holm family (`holm`); the pass set here is the
        # unadjusted one, until `holm` runs over the family
        add(what, t, held and t["p_value"] > ALPHA, held=bool(held), tested=True, **kw)

    so, sn = sum(o["swings"].values()), sum(n["swings"].values())
    # a required sample (`min_swings`) always gets its row: no swings at all fails it
    if so or sn or min_swings > 0:
        required = min_swings > 0
        need = min_swings if required else MIN_SWINGS
        enough = so >= need and sn >= need
        lo = so - o["swings"]["miss"]
        ln = sn - n["swings"]["miss"]
        row = dict(orig=[lo, so], new=[ln, sn], bound=HIT_RATE_BOUND, min_swings=need,
                   enough=enough, required=required,
                   **short_note(so, sn, need, required, enough, HIT_RATE_BOUND))
        if so and sn:
            diff = lo / so - ln / sn
            # the 2-point bound on the player's rows (John, 2026-10-17, option C)
            held = abs(diff) <= HIT_RATE_BOUND if enough else not required
            tested("hit rate", two_proportion_z(lo, so, ln, sn), held, difference=diff, **row)
        else:
            add("hit rate", test(0.0, 0, 0.0), False, difference=1.0, **row)
        tested("outcome mix",
               chi_square_homogeneity(_vec(o["swings"], OUTCOMES), _vec(n["swings"], OUTCOMES)),
               orig=_vec(o["swings"], OUTCOMES), new=_vec(n["swings"], OUTCOMES), bins=OUTCOMES)
    mo, mn = o.get("mon_swings", collections.Counter()), n.get("mon_swings", collections.Counter())
    smo, smn = sum(mo.values()), sum(mn.values())
    if smo or smn or (min_swings > 0 and monster_required):
        # the monster sheet's swings at the player: the same sample as the player's (John,
        # 2026-10-16: N >= 2,000 swings per sheet), but no 2-point bound: only a significant
        # difference fails it (John, 2026-10-17, option C)
        required = min_swings > 0
        need = min_swings if required else MIN_SWINGS
        enough = smo >= need and smn >= need
        lo, ln = smo - mo["miss"] - mo["dodge"], smn - mn["miss"] - mn["dodge"]
        row = dict(orig=[lo, smo], new=[ln, smn], min_swings=need, enough=enough,
                   required=required, **short_note(smo, smn, need, required, enough))
        if smo and smn:
            tested("monster hit rate", two_proportion_z(lo, smo, ln, smn),
                   enough or not required, difference=lo / smo - ln / smn, **row)
        else:
            add("monster hit rate", test(0.0, 0, 0.0), False, difference=1.0, **row)
        tested("monster outcome mix",
               chi_square_homogeneity(_vec(mo, MON_OUTCOMES), _vec(mn, MON_OUTCOMES)),
               orig=_vec(mo, MON_OUTCOMES), new=_vec(mn, MON_OUTCOMES), bins=MON_OUTCOMES)
        dmo, dmn = o.get("mon_damage", {}), n.get("mon_damage", {})
        keys = sorted(set(dmo) | set(dmn))
        if keys:
            tested("monster damage histogram",
                   chi_square_homogeneity(_vec(dmo, keys), _vec(dmn, keys)), bins=keys,
                   orig=_vec(dmo, keys), new=_vec(dmn, keys))
    keys = sorted(set(o["damage"]) | set(n["damage"]))
    if keys:
        tested("damage histogram",
               chi_square_homogeneity(_vec(o["damage"], keys), _vec(n["damage"], keys)),
               bins=keys, orig=_vec(o["damage"], keys), new=_vec(n["damage"], keys))
    keys = sorted(set(o["blows"]) | set(n["blows"]))
    if keys:
        tested("blows per round",
               chi_square_homogeneity(_vec(o["blows"], keys), _vec(n["blows"], keys)),
               bins=keys, orig=_vec(o["blows"], keys), new=_vec(n["blows"], keys))
    mro, mrn = o.get("mon_rounds", {}), n.get("mon_rounds", {})
    keys = sorted(set(mro) | set(mrn))
    if keys:
        # the monster's swings at the player in each round the player swung in: how many
        # monsters attack, and whether they keep attacking (a few hundred rounds suffice)
        # Advisory (John, 2026-10-17): an MBBSEmu artifact, not a gap. MBBSEmu's farcoreleft
        # reads -1, so every record cache starts at its floor (Active Monsters 21); which monsters a
        # 21-slot cache evicts, and so how often a sheet monster misses its energy refill
        #, follows a monster numbering silvermere cannot reproduce, and the
        # original's maxactive reads 27 (the cold-start value; why the
        # restart keeps it is under investigation, MMUDACT.LOG captures `Max Active:`) (runs 38998045779, 38998127086, 39002652491,
        # 39003804839, 39015321649). The row stays in the report, flagged at p <= 0.01.
        mean = lambda c: sum(k * v for k, v in c.items()) / max(sum(c.values()), 1)  # noqa: E731
        t = chi_square_homogeneity(_vec(mro, keys), _vec(mrn, keys))
        add("monster swings per round", t, True, advisory=True, flag=t["p_value"] <= ALPHA,
            bins=keys, orig=_vec(mro, keys), new=_vec(mrn, keys), mean=[mean(mro), mean(mrn)],
            note=MON_ROUNDS_NOTE)
    if o["regen"] or n["regen"] or min_regen:
        same = set(o["regen"]) == set(n["regen"])
        need = max(min_regen, REGEN_SAMPLE)
        enough = sum(o["regen"].values()) >= need and sum(n["regen"].values()) >= need
        extra = {}
        top = (o.get("regen_cap_from_top", 0), n.get("regen_cap_from_top", 0))
        if any(top):
            extra["note"] = (f"no `st` Hits line for some gains (orig {top[0]}, new {top[1]}): "
                             "maxhp taken as the highest hit points the run shows")
        if not enough and min_regen:
            short = (f"sample short (orig {sum(o['regen'].values())}, new "
                     f"{sum(n['regen'].values())} gains; {need} needed a side), not compared")
            extra["note"] = "; ".join(x for x in (extra.get("note"), short) if x)
        add("regen per tick", test(0.0, 0, 1.0 if same else 0.0),
            same if enough else not min_regen,
            orig=sorted(o["regen"]), new=sorted(n["regen"]), enough=enough, min_gains=need, **extra)
    to, tn = durations
    # (what, sample key, gated): spawns are gated; walk-ins only in a scenario that counts them
    # over a fixed time (`gate_wanders`, the arena watch: John, 2026-10-16, spawn and wander
    # counts; T-086); elsewhere walk-ins and monster noise are advisory, flagged at p <= 0.01
    # (John: arena crowding is not the spawn check). `gate_wanders` also requires both rows.
    for what, k, gated in (("spawns per minute", "arrivals", True),
                           ("wanderers per minute", "wanders", gate_wanders),
                           ("monster noise per minute", "noise", False)):
        x, y = o.get(k, 0), n.get(k, 0)
        if not (x or y or (gate_wanders and k != "noise")):
            continue
        rates = dict(orig=[x, to], new=[y, tn])
        short = [side for side, d in (("orig", to), ("new", tn)) if d < MIN_RATE_SECONDS]
        if short:
            # a side that did not watch (it never got in, or ended at once) is no sample: the
            # row refuses rather than compares a rate against nothing
            t = test(0.0, 0, 0.0)
            msg = f"no sample on {', '.join(short)} ({MIN_RATE_SECONDS:.0f} s needed)"
            if gated:
                add(what, t, False, no_sample=short, note=msg, **rates)
            else:
                add(what, t, True, advisory=True, flag=True, no_sample=short, note=msg, **rates)
            continue
        if gated and not (x or y):
            # nothing happened on either side: a rate of 0 against 0 says nothing about the
            # pass it is meant to check, so the gated row refuses (review of #72: the arena
            # watch's 0/0 walk-ins passed as if compared)
            add(what, test(0.0, 0, 0.0), False, no_events=True,
                note="no events on either side, not compared", **rates)
            continue
        t = poisson_rates(x, to, y, tn)
        rates["rate_per_minute"] = [60 * x / to, 60 * y / tn]
        if gated:
            tested(what, t, **rates)
        else:
            add(what, t, True, advisory=True, flag=t["p_value"] <= ALPHA, **rates)
    return checks


def against_closed_form(side, case):
    """One side's swings against a case of ab_sheets.json (the Rust export of the closed form):
    the hit rate by z-test and the hit damage by goodness of fit."""
    checks = []
    total = sum(side["swings"].values())
    if total and "hit" in case.get("rates", {}):
        landed = total - side["swings"]["miss"]
        p = case["rates"]["hit"]["expected"]
        t = binomial_z(landed, total, p)
        checks.append(dict({"what": f"hit rate vs closed form ({case['case']})",
                            "pass": t["p_value"] > ALPHA, "observed": [landed, total],
                            "expected": p, "tested": True, "held": True}, **t))
    hits = {int(k.split(":")[1]): v for k, v in case.get("expected", {}).items()
            if k.startswith("hit:")}
    if hits and side["damage"]:
        keys = sorted(hits)
        s = sum(hits.values())
        t = chi_square_gof(_vec(side["damage"], keys), [hits[k] / s for k in keys])
        checks.append(dict({"what": f"damage vs closed form ({case['case']})",
                            "pass": t["p_value"] > ALPHA, "tested": True, "held": True}, **t))
    return checks


# ---- record level --------------------------------------------------------------------------

PLAYER_FIELDS = ["hp", "maxhp", "exp", "level", "cp", "lives", "str", "intel", "wil", "agi",
                 "hea", "cha", "copper", "silver", "gold", "platinum", "runic", "settings",
                 "statline", "msglevel"]


def _slots(items, uses):
    return sorted((i, u) for i, u in zip(items, uses) if i)


def orig_player(rec):
    """The comparable fields of a decoded WCCUSERS record (mmud_data.decode_record)."""
    out = {k: rec[k] for k in PLAYER_FIELDS if k in rec}
    out["inventory"] = _slots(rec.get("items", []), rec.get("itemuses", []))
    out["worn"] = _slots(rec.get("worn", []), rec.get("wornuses", []))
    return out


def new_player(conn, name):
    """The same fields of the new server's `characters` row (SQLite)."""
    conn.row_factory = sqlite3.Row
    row = conn.execute("select * from characters where lower(firstname) = lower(?)",
                       (name,)).fetchone()
    if row is None:
        return None
    out = {k: row[k] for k in PLAYER_FIELDS if k in row.keys()}
    for table, key in (("character_inventory", "inventory"), ("character_worn", "worn")):
        rows = conn.execute(f"select item, uses from {table} where character_id = ? and item != 0",
                            (row["id"],)).fetchall()
        out[key] = sorted((r["item"], r["uses"]) for r in rows)
    return out


def acmsr_summary(nums):
    """WCCACMSR's active-monster numbers in brief: how many, the lowest and highest, the gaps
    below the highest (the numbers a cold start would free), and how many lie above 27, the
    original's active-monster limit after a cold start (#72)."""
    nums = sorted(nums)
    if not nums:
        return {"count": 0}
    return {"count": len(nums), "min": nums[0], "max": nums[-1],
            "gaps_below_max": nums[-1] - len(set(nums)), "above_27": sum(n > 27 for n in nums)}


# Not carried by design, so not compared (#72, run 39030247260):
# - `PS_PAYING` (0x8000): the original sets it from the SAVEKEY key at creation
#   and at each entry; SAVEKEY is dropped (docs/auth.md,
#   newchar.rs, player/save.rs), so silvermere never sets it.
# - a worn item's uses: on the original an item worn with EQUIP gets an arbitrary value
#   (23 under MBBSEmu), and silvermere stores EQUIP's own lookup (items/equip.rs, -1 for no charges). The worn item
#   numbers are still compared.
PS_PAYING = 0x8000


def comparable(rec):
    """A player's fields with what is not carried by design taken out (above)."""
    if rec is None:
        return None
    out = dict(rec)
    if out.get("settings") is not None:
        out["settings"] &= ~PS_PAYING
    if "worn" in out:
        out["worn"] = sorted(i for i, _ in out["worn"])
    return out


def compare_records(orig, new, ignore=()):
    """[{field, orig, new}] where the two differ, `ignore` left out (after random fights hp,
    exp and coins differ by chance; the rest must not), and what is not carried by design
    (`comparable`) taken out."""
    if orig is None or new is None:
        return [{"field": "*", "orig": orig is not None, "new": new is not None}]
    orig, new = comparable(orig), comparable(new)
    return [{"field": k, "orig": orig.get(k), "new": new.get(k)}
            for k in sorted(set(orig) | set(new))
            if k not in ignore and orig.get(k) != new.get(k)]


def monsters_per_room(rows):
    """Counter of (map, room) over active-monster records with a number."""
    return collections.Counter((r["map"], r["room"]) for r in rows if r.get("number"))


def compare_monsters(orig_rows, new_rows):
    o, n = monsters_per_room(orig_rows), monsters_per_room(new_rows)
    return [{"room": f"{m}:{r}", "orig": o.get((m, r), 0), "new": n.get((m, r), 0)}
            for m, r in sorted(set(o) | set(n)) if o.get((m, r), 0) != n.get((m, r), 0)]


def monster_maps(orig_rows, new_rows):
    """Active monsters per map on both sides: (all maps, the gaps). A gap is a map with
    monsters on one side and none on the other, a systematic difference (the start-up fill of
    every Lair and Shop and every room's specific monster, for one). Per-room
    counts are not a gate: monsters wander, so they drift by chance."""
    o, n = collections.Counter(), collections.Counter()
    for (m, _), k in monsters_per_room(orig_rows).items():
        o[m] += k
    for (m, _), k in monsters_per_room(new_rows).items():
        n[m] += k
    maps = [{"map": m, "orig": o.get(m, 0), "new": n.get(m, 0)} for m in sorted(set(o) | set(n))]
    return maps, [x for x in maps if (x["orig"] == 0) != (x["new"] == 0)]


def load_orig_records(data_dir, name):
    """(player, active monsters) from MBBSEmu's WCCUSERS.db and WCCACMSR.db in data_dir."""
    vendor_play = os.path.join(HERE, "vendor", "play")
    if vendor_play not in sys.path:
        sys.path.insert(0, vendor_play)
    import mmud_data as md
    player = None
    for _, d in md.read_rows(data_dir, "WCCUSERS") or []:
        rec = md.decode_record(md.RECORDS["WCCUSERS"], d)
        if rec.get("firstname", "").lower() == name.lower():
            player = orig_player(rec)
    mons = [md.decode_record(md.RECORDS["WCCACMSR"], d)
            for _, d in md.read_rows(data_dir, "WCCACMSR") or []]
    return player, mons


def load_new_records(db_path, name):
    conn = sqlite3.connect(db_path)
    try:
        player = new_player(conn, name)
        conn.row_factory = sqlite3.Row
        mons = [dict(r) for r in conn.execute("select number, map, room from active_monsters")]
    finally:
        conn.close()
    return player, mons


# ---- the harness hook and the report -----------------------------------------------------

def expand_script(script, m2):
    """The script with `m2.repeat` applied: steps first..last (inclusive) played `times` times in
    a row (the fight cycle of m2/fight-loop), so the stored script holds one cycle."""
    rep = (m2 or {}).get("repeat")
    if not rep:
        return script
    steps = script.get("steps", [])
    a, b, times = rep["first"], rep["last"], rep["times"]
    # a shard of a long run (a host that restarts every hour or so): fewer cycles per run,
    # each shard with its own report dir, pooled by `stats.py report`
    if os.environ.get("SILVERMERE_M2_REPEAT"):
        times = int(os.environ["SILVERMERE_M2_REPEAT"])
    out = dict(script)
    # `#CYCLE#` in a step becomes the cycle's number, so a variable `set` in one cycle (a
    # crowded arena) does not carry over to the next: a script cannot unset a variable
    body = json.dumps(steps[a:b + 1])
    cycles = [s for k in range(times) for s in json.loads(body.replace("#CYCLE#", str(k)))]
    out["steps"] = steps[:a] + cycles + steps[b + 1:]
    return out


def round_blows(steps, rounds, registry, monster=None, mon_rounds=None):
    """Blows per round: the player's swings in each `round` step's own bytes (a sleep, so the
    events of the whole stream would carry the step of the send before it), classified alone;
    a window without a swing (the fight over, the target gone) is no round. `mon_rounds`, when
    given, gets the monster's swings at the player in each of those rounds (the #72 review,
    item 6: monsters that attack every round on one side and skip rounds on the other)."""
    import normalise
    out = collections.Counter()
    for st in steps:
        if st.index in rounds and st.raw:
            evs = normalise.events_for(st.raw, [(st.index, 0, None)], {}, registry)
            n = sum(1 for e in evs if e["name"] in SWING and _of_monster(e.get("fields", {}), monster))
            if n:
                out[n] += 1
                if mon_rounds is not None:
                    mon_rounds[sum(1 for e in evs if e["name"] in MON_SWING
                                   and _of_monster(e.get("fields", {}), monster))] += 1
    return out


BUFFERS_RX = re.compile(r"Active Monsters: +Size: +(\d+) +Used: +(\d+) +Hits: +(\d+) +Miss: +(\d+)")


def buffer_lines(steps):
    """The active-monster cache as SYSOP BUFFERS showed it at the first and the last call of a
    run: `Size/Used/Hits/Miss` (display_buffer_statistics; silvermere's
    version since T-101)."""
    import replay
    bridge, _, _ = replay.load_play()
    text = bridge.plain(b"".join(st.raw for st in steps).decode("cp437", "replace")) if steps else ""
    found = []
    for m in BUFFERS_RX.finditer(text):
        found.append(f"size {m.group(1)} used {m.group(2)} hits {m.group(3)} miss {m.group(4)}")
    return [found[0], found[-1]] if found else []


def round_steps(script):
    """The steps a script marks `"round": true`: waits of one energy round inside a fight."""
    return [i for i, st in enumerate(script.get("steps", [])) if st.get("round")]


def dump_samples(side):
    """A samples() dict as JSON (counter keys as strings)."""
    return {k: ({str(x): n for x, n in v.items()} if isinstance(v, dict) else v)
            for k, v in side.items()}


def load_samples(doc):
    """dump_samples() back: swing outcomes keep string keys, the rest are integers."""
    out = {}
    for k, v in doc.items():
        if isinstance(v, dict):
            out[k] = collections.Counter({(x if k in ("swings", "mon_swings") else int(x)): n
                                          for x, n in v.items()})
        else:
            out[k] = v
    for k in ("swings", "damage", "blows", "regen", "mon_swings", "mon_damage", "mon_rounds"):
        out.setdefault(k, collections.Counter())
    out.setdefault("arrivals", 0)
    out.setdefault("wanders", 0)
    out.setdefault("noise", 0)
    return out


def pool_samples(sides):
    """The sum of several samples() dicts (shards of one scenario: no fight is in two)."""
    out = load_samples({})
    for s in sides:
        for k in ("swings", "damage", "blows", "regen", "mon_swings", "mon_damage", "mon_rounds"):
            out[k].update(s.get(k, {}))
        out["arrivals"] += s.get("arrivals", 0)
        out["wanders"] += s.get("wanders", 0)
        out["noise"] += s.get("noise", 0)
        out["regen_cap_from_top"] = (out.get("regen_cap_from_top", 0)
                                     + s.get("regen_cap_from_top", 0))
    return out


def checks_of(m2, orig, new, durations, ab=None, min_swings=MIN_SWINGS):
    if m2.get("regen") is False:
        # the sheet runs reset the hit points every cycle (TWEAK HP): no regen sample
        orig, new = dict(orig, regen=collections.Counter()), dict(new, regen=collections.Counter())
    need = m2.get("min_swings", min_swings)
    if check_run():
        # a check with the cap lowered: the 2,000-swing sample cannot be in, so it is not
        # required; the tests (Holm) still decide, and the player's bound applies from
        # MIN_SWINGS a side
        need = 0
    checks = compare_samples(orig, new, tuple(durations), need,
                             m2.get("min_regen", 0), m2.get("gate_wanders", False),
                             bool(m2.get("monster")))
    if ab and m2.get("sheet"):
        case = next((c for c in ab["cases"] if c["case"] == m2["sheet"]), None)
        if case:
            checks += against_closed_form(new, case)
    checks = scope_checks(m2, checks)
    holm(checks)   # the scenario's own family; `collect` runs it again over the whole report
    return checks


def check_run():
    """`SILVERMERE_M2_MINUTES` set: a check run with the time cap lowered (m2_live.sh), whose
    samples are short by design."""
    return bool(os.environ.get("SILVERMERE_M2_MINUTES"))


RATE_ROWS = {"spawns per minute", "wanderers per minute", "monster noise per minute"}
SWING_ROWS = {"hit rate", "outcome mix", "damage histogram", "blows per round",
              "monster swings per round",
              "monster hit rate", "monster outcome mix", "monster damage histogram"}


def scope_checks(m2, checks):
    """Which rows a scenario gates (live run 38953732059):
    - rates: `m2.rates` true (the arena watch) gates them; false (the sheet runs) drops them,
      since there the walk-ins and noise are the summoned monsters, as many per swing on both
      sides and so a measure of how fast each server plays, not of the wander pass; unset (the
      short scenarios, whose watches end when a monster comes) leaves them advisory;
    - swings: a scenario with no sheet (`m2.monster`) and no swing sample (`min_swings` 0)
      fights whatever comes, a mix of monsters that differs between the sides by chance, so its
      swing rows are advisory: the sheet runs are the swing comparison."""
    rates = m2.get("rates")
    out = []
    for c in checks:
        what = c["what"]
        if what in RATE_ROWS and rates is False:
            continue
        advisory = ((what in RATE_ROWS and rates is None)
                    or (what in SWING_ROWS and (m2.get("watch") or not m2.get("monster"))
                        and not m2.get("min_swings"))
                    # the wander watch gates its walk-ins only
                    or (m2.get("watch") and what in RATE_ROWS and what != "wanderers per minute"))
        if advisory and not c.get("advisory"):
            c = dict(c, advisory=True, flag=not c["pass"], **{"pass": True})
        out.append(c)
    return out


def scenario_stats(scenario, script, orig_steps, new_steps, ab=None, min_swings=MIN_SWINGS,
                   registry=None):
    """{"checks", "samples"} of one scenario: the checks, and both sides' raw counts with the
    durations and why each side stopped (`stop`: its sample was in, or its time was up), which
    summary.json keeps so `stats.py report` can pool shards. `registry`: the classifier the
    harness used (with the game-data patterns for the monsters' texts)."""
    import normalise
    import replay
    if registry is None:
        _, _, pt = replay.load_play()
        registry = pt.Registry.load(data=False)
    rs = round_steps(script)
    sides, durations, stops = [], [], []
    m2 = scenario.get("m2") or {}
    monster = m2.get("monster")
    # the wander watch: its `watch` steps (the send that starts a watch, the sleep after it)
    watch = ({i for i, st in enumerate(script.get("steps", [])) if st.get("watch")}
             if m2.get("watch") else None)
    for steps in (orig_steps, new_steps):
        events = normalise.events_of(steps, {}, registry) if steps else []
        side = samples(events, (), monster, watch)
        side["mon_rounds"] = collections.Counter()
        side["blows"] = round_blows(steps, set(rs), registry, monster, side["mon_rounds"])
        sides.append(side)
        if watch is not None:
            # a rate per minute of watching
            durations.append(sum(max(st.waited or 0.0, 0.0) for st in steps if st.index in watch))
        else:
            durations.append(sum(max(st.waited or 0.0, 0.0) for st in steps))
        stops.append(getattr(steps[0], "stop_info", None) if steps else None)
        side["buffers"] = buffer_lines(steps)
        if watch is None and stops[-1] and stops[-1].get("elapsed"):
            # the run's own clock from its first cycle to its stop: the steps' waits leave out
            # the time between them (the arena watch: 1,005 s of waits in a 1,500 s watch)
            durations[-1] = float(stops[-1]["elapsed"])
    checks = checks_of(scenario.get("m2", {}), sides[0], sides[1], durations, ab, min_swings)
    return {"checks": checks,
            "samples": {"orig": dump_samples(sides[0]), "new": dump_samples(sides[1]),
                        "durations": durations, "stop": stops}}


def scenario_checks(scenario, script, orig_steps, new_steps, ab=None, min_swings=MIN_SWINGS):
    """The statistical checks of one scenario from both sides' StepCaptures: events unmasked
    through the registry, the durations from the steps' waits."""
    return scenario_stats(scenario, script, orig_steps, new_steps, ab, min_swings)["checks"]


def load_ab(path=None):
    """ab_sheets.json: `$SILVERMERE_AB_EXPORT`, or the path given; None when absent."""
    path = path or os.environ.get("SILVERMERE_AB_EXPORT")
    if not path or not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def collect(report_dirs, group="m2", ab=None):
    """The m2 statistics of one or more harness reports. One report: the `stats` each scenario's
    summary.json carries. Several (shards of a long run, `SILVERMERE_M2_REPEAT`): for each
    scenario the raw counts of every shard are added up and the checks run once on the sums."""
    if isinstance(report_dirs, str):
        report_dirs = [report_dirs]
    found = {}
    for d in report_dirs:
        base = os.path.join(d, group)
        for leaf in sorted(os.listdir(base)) if os.path.isdir(base) else []:
            path = os.path.join(base, leaf, "summary.json")
            if os.path.exists(path):
                with open(path, encoding="utf-8") as f:
                    found.setdefault(leaf, []).append(json.load(f))
    summary = {"group": group, "alpha": ALPHA, "reports": list(report_dirs), "scenarios": [],
               "check_run": check_run()}
    for leaf in sorted(found):
        results = found[leaf]
        name = f"{group}/{leaf}"
        entry = {"scenario": name, "shards": len(results),
                 "harness": [r.get("status") for r in results]}
        if len(results) > 1 and all(r.get("samples") for r in results):
            m2 = {}
            try:
                from differential import load_scenario
                m2 = load_scenario(name).get("m2", {})
            except (OSError, SystemExit, ValueError):
                pass
            o = pool_samples(load_samples(r["samples"]["orig"]) for r in results)
            n = pool_samples(load_samples(r["samples"]["new"]) for r in results)
            durations = [sum(r["samples"]["durations"][i] for r in results) for i in (0, 1)]
            checks = checks_of(m2, o, n, durations, ab)
            entry["pooled"] = True
        else:
            checks = [c for r in results for c in r.get("stats", [])]
        entry["checks"] = checks
        entry["sample_sizes"] = sample_sizes(results)
        entry["buffers"] = {side: [r["samples"][side].get("buffers") for r in results
                                   if r.get("samples") and r["samples"][side].get("buffers")]
                            for side in ("orig", "new")}
        summary["scenarios"].append(entry)
    # Holm over every gated row of the report (John, 2026-10-17); a row's raw p is kept
    summary["holm_family"] = holm([c for s in summary["scenarios"] for c in s["checks"]])
    for s in summary["scenarios"]:
        s["pass"] = all(c["pass"] for c in s["checks"])
    summary["pass"] = all(s["pass"] for s in summary["scenarios"])
    return summary


def sample_sizes(results):
    """Per side, what the scenario's runs (one, or the parallel sessions and shards) gathered:
    swings, the monsters' swings at the player, spawns, walk-ins, seconds played, and how each
    session stopped (its sample in, or its time up, and how far it got)."""
    out = {}
    for i, side in enumerate(("orig", "new")):
        docs = [r["samples"] for r in results if r.get("samples")]
        sm = pool_samples(load_samples(d[side]) for d in docs)
        out[side] = {"swings": sum(sm["swings"].values()),
                     "monster swings": sum(sm["mon_swings"].values()),
                     "spawns": sm["arrivals"], "walk-ins": sm["wanders"],
                     "seconds": round(sum(d["durations"][i] for d in docs), 1),
                     "sessions": len(docs),
                     "stopped": [(d.get("stop") or [None, None])[i] for d in docs]}
    return out


def write_summary(report_dir, summary):
    with open(os.path.join(report_dir, "stats.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=1, sort_keys=True)
    lines = [f"# Statistical comparison ({summary['group']})", "",
             f"family alpha {summary['alpha']}, Holm-Bonferroni over the "
             f"{summary.get('holm_family', 0)} gated tested rows (John, 2026-10-17); "
             f"overall: {'PASS' if summary['pass'] else 'FAIL'}", ""]
    if summary.get("check_run"):
        lines += ["Check run (SILVERMERE_M2_MINUTES): the 2,000-swing samples are not "
                  "required; this is not the M2 verdict.", ""]
    lines += ["A gated row fails when Holm rejects it (its adjusted p at or under alpha; the "
              "step-down stops at the first raw p above its threshold), or when its other "
              "condition fails: a player hit rate's 2-point bound, a short required sample, a "
              "rate with no sample or no events.", "",
              "| scenario | check | orig | new | statistic | df | p | Holm threshold "
              "| adjusted p | result |",
              "|---|---|---|---|---|---|---|---|---|---|"]
    for s in summary["scenarios"]:
        for c in s["checks"]:
            if "holm_alpha" in c:
                th, adj = f"{c['holm_alpha']:.5f}", f"{c['holm_p']:.4f}"
            else:
                th = adj = "-"
            result = (("FLAG" if c.get("flag") else "advisory") if c.get("advisory")
                      else "pass" if c["pass"] else "FAIL")
            if not c["pass"] and c.get("tested"):
                why = [w for w, bad in (("Holm rejects", c.get("rejected")),
                                        ("bound or sample", not c.get("held", True))) if bad]
                result += f" ({', '.join(why)})" if why else ""
            lines.append(f"| {s['scenario']} | {c['what']} | {c.get('orig', c.get('observed'))} "
                         f"| {c.get('new', c.get('expected'))} | {c['statistic']:.3f} | {c['df']} "
                         f"| {c['p_value']:.4f} | {th} | {adj} "
                         f"| {result}{(': ' + c['note']) if c.get('note') else ''} |")
    rows = [(s["scenario"], side, b) for s in summary["scenarios"]
            for side, b in (s.get("buffers") or {}).items() if b]
    init = summary.get("orig_init")
    if rows or init:
        lines += ["", "## Active-monster cache (SYSOP BUFFERS, first and last call per session)", ""]
        if init:
            # the original's start-up cycle before play (SYSOP INITIALIZE -1, m2_live.sh)
            lines += [("orig start-up cycle " + init) if init.startswith("not finished")
                      else f"orig start-up cycle {init}", ""]
        lines += ["| scenario | side | start / end |", "|---|---|---|"]
        lines += [f"| {sc} | {side} | {'; '.join(' / '.join(x) for x in b)} |" for sc, side, b in rows]
    lines += ["", "## Sample sizes", "",
              "| scenario | side | sessions | swings | monster swings | spawns | walk-ins "
              "| seconds | stopped |", "|---|---|---|---|---|---|---|---|---|"]
    for s in summary["scenarios"]:
        for side, z in (s.get("sample_sizes") or {}).items():
            why = collections.Counter(
                (x or {}).get("reason", "end of script") for x in z["stopped"])
            stopped = ", ".join(f"{k} x{v}" for k, v in sorted(why.items()))
            lines.append(f"| {s['scenario']} | {side} | {z['sessions']} | {z['swings']} "
                         f"| {z['monster swings']} | {z['spawns']} | {z['walk-ins']} "
                         f"| {z['seconds']} | {stopped} |")
    with open(os.path.join(report_dir, "stats.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


# ---- self-test: the numbers stats.rs pins --------------------------------------------------

def selftest():
    close = lambda a, b, t: abs(a - b) <= t  # noqa: E731
    assert close(chi_square_sf(3.841459, 1), 0.05, 1e-6)
    assert close(chi_square_sf(9.210340, 2), 0.01, 1e-6)
    assert close(chi_square_sf(23.20925, 10), 0.01, 1e-6)
    assert close(normal_two_sided(1.959964), 0.05, 1e-6)
    assert close(ln_gamma(5.0), math.log(24.0), 1e-12)
    assert pool([1.0, 1.0, 10.0, 2.0], [2.0, 4.0, 10.0, 1.0]) == [(2.0, 6.0), (12.0, 11.0)]
    assert close(chi_square_gof([160, 80, 80, 80], [0.25] * 4)["statistic"], 48.0, 1e-9)
    assert chi_square_homogeneity([50, 30, 20], [100, 60, 40])["df"] == 2
    assert close(two_proportion_z(620, 1000, 600, 1000)["statistic"], 0.916891, 1e-5)
    print("stats.py selftest: ok")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("selftest")
    c = sub.add_parser("report")
    c.add_argument("--report", required=True, action="append",
                   help="a harness report dir; repeat it for the shards of one long run")
    c.add_argument("--scenarios", default="m2")
    c.add_argument("--out", help="where stats.md/stats.json go (default the first --report)")
    c.add_argument("--orig-init", metavar="FILE",
                   help="m2_live.sh's line on the original's start-up cycle (finished or not)")
    q = sub.add_parser("acmsr", help="the original's WCCACMSR: count, lowest and highest number")
    q.add_argument("--orig-db", required=True, help="the MBBSEmu module data directory (WCC*.db)")
    r = sub.add_parser("records")
    r.add_argument("--orig-db", required=True, help="the MBBSEmu module data directory (WCC*.db)")
    r.add_argument("--new-db", required=True, help="the new server's SQLite file")
    r.add_argument("--name", required=True)
    r.add_argument("--ignore", default="",
                   help="comma-separated fields left out, e.g. hp,exp,copper,silver,gold after "
                        "random fights")
    a = ap.parse_args(argv)
    if a.cmd == "selftest":
        selftest()
        return 0
    if a.cmd == "acmsr":
        _, mons = load_orig_records(a.orig_db, "")
        nums = sorted(m["number"] for m in mons if m.get("number"))
        print(json.dumps(acmsr_summary(nums)))
        return 0
    if a.cmd == "records":
        op, om = load_orig_records(a.orig_db, a.name)
        np_, nm = load_new_records(a.new_db, a.name)
        ignore = {f for f in a.ignore.split(",") if f}
        maps, gaps = monster_maps(om, nm)
        rooms = compare_monsters(om, nm)
        out = {"player": compare_records(op, np_, ignore), "monster_gaps": gaps,
               "monster_maps": maps, "monster_rooms_differing": len(rooms),
               "monster_rooms": rooms[:20]}
        print(json.dumps(out, indent=1))
        return 0 if not (out["player"] or out["monster_gaps"]) else 1
    summary = collect(a.report, a.scenarios, load_ab())
    if a.orig_init and os.path.exists(a.orig_init):
        with open(a.orig_init, encoding="utf-8") as f:
            summary["orig_init"] = f.read().strip()
    out = a.out or a.report[0]
    os.makedirs(out, exist_ok=True)
    write_summary(out, summary)
    print(f"stats: {'PASS' if summary['pass'] else 'FAIL'} "
          f"({len(summary['scenarios'])} scenarios); see {out}/stats.md")
    return 0 if summary["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
