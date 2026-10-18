"""Writes report/<scenario>/{orig.raw,new.raw,orig.events.jsonl,new.events.jsonl,diff.txt,
summary.json} and the top-level summary.md."""
import json
import os

from normalise import write_jsonl


def write_scenario(report_dir, name, result, orig_steps, new_steps, orig_events, new_events, diff_text):
    d = os.path.join(report_dir, name)
    os.makedirs(d, exist_ok=True)
    for side, steps in (("orig", orig_steps), ("new", new_steps)):
        with open(os.path.join(d, f"{side}.raw"), "wb") as f:
            for s in steps:
                f.write(s.raw)
    write_jsonl(os.path.join(d, "orig.events.jsonl"), orig_events)
    write_jsonl(os.path.join(d, "new.events.jsonl"), new_events)
    with open(os.path.join(d, "diff.txt"), "w", encoding="utf-8") as f:
        f.write(diff_text + ("\n" if diff_text else ""))
    with open(os.path.join(d, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(result, f, indent=1, sort_keys=True)
        f.write("\n")


def gaps_cell(gaps):
    """`T-083: 0/0 +0/7; T-055: 1/1` for the summary table."""
    out = []
    for card, g in sorted(gaps.items()):
        cell = f"{card}: {g.get('orig', 0)}/{g.get('new', 0)}"
        if g.get("orig_bytes") or g.get("new_bytes"):
            cell += f" +{g.get('orig_bytes', 0)}/{g.get('new_bytes', 0)}"
        out.append(cell)
    return "; ".join(out)


def redraws_cell(redraws):
    """`3/4` (events, bytes and rejoined lines, orig/new) for the summary table."""
    if not redraws:
        return ""
    n = {side: redraws[side]["events"] + redraws[side]["bytes"] + redraws[side].get("rejoined", 0)
         for side in ("orig", "new")}
    return f"{n['orig']}/{n['new']}"


def write_summary(report_dir, results):
    os.makedirs(report_dir, exist_ok=True)
    lines = ["# Differential report", "",
             "| Scenario | Result | Event diffs | Unknown (new only) | Byte-equal steps | Known gaps "
             "(card: orig/new events, +bytes) | Prompt redraws collapsed (orig/new) | Problems | Notes |",
             "|---|---|---|---|---|---|---|---|---|"]
    for r in results:
        if r["status"] == "skipped":
            lines.append(f"| {r['scenario']} | SKIP | | | | | | {r['reason']} | |")
            continue
        det = r["byte"]
        lines.append(f"| {r['scenario']} | {r['status'].upper()} | {r['event_diffs']} | "
                     f"{len(r['unknown_extra'])} | {det['equal']}/{det['checked']} | "
                     f"{gaps_cell(r.get('known_gaps', {}))} | {redraws_cell(r.get('redraws'))} | "
                     f"{'; '.join(r['problems']) or ''} | "
                     f"{'; '.join(r.get('notes', []))} |")
    failed = sum(1 for r in results if r["status"] == "fail")
    rerun = sum(1 for r in results if r.get("rerun"))
    lines += ["", f"{len(results)} scenarios, {failed} failed, {rerun} rerun after interference."]
    with open(os.path.join(report_dir, "summary.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
