#!/usr/bin/env python3
"""Quantify 1994 Old School playtest progress (transient reporting tool).

For each deck-tracking issue ("TRACK: Old School 1994 deck ..." or
"Deck Compatibility: ...") parse the per-card issue IDs referenced in its
description and report how many are CLOSED (= card reached WORKING). Also
prints the overall 'Card Compatibility:' closed/total.

Robust: uses `mb list --json` (structured) rather than scraping human output,
so it is immune to terminal formatting / color / row-limit differences.

Usage:  scripts/temp/oldschool_progress.py        (run from anywhere in the repo)
Requires: mb (Minibeads) on PATH.
"""
import json
import re
import subprocess
import sys
import datetime


def sh(args):
    return subprocess.run(args, capture_output=True, text=True, check=True).stdout


def main():
    try:
        raw = sh(["mb", "list", "--json", "--limit", "100000"])
    except FileNotFoundError:
        sys.exit("error: mb (Minibeads) not on PATH")
    except subprocess.CalledProcessError as e:
        sys.exit(f"error: `mb list --json` failed: {e.stderr.strip()}")
    issues = json.loads(raw)
    if not issues:
        sys.exit("error: mb returned no issues (run from inside the repo / its .beads).")

    by_id = {i["id"]: i for i in issues}
    ref_re = re.compile(r"mtg-[0-9a-z]+")

    def is_deck(i):
        return i.get("title", "").startswith(
            ("TRACK: Old School 1994 deck", "Deck Compatibility:"))

    def clean(title):
        t = re.sub(r"^TRACK: Old School 1994 deck ", "", title)
        t = re.sub(r"^Deck Compatibility: ", "", t)
        t = re.sub(r" — full compatibility$", "", t)
        return t.strip().strip("'")

    decks = sorted((i for i in issues if is_deck(i)), key=lambda i: clean(i["title"]))

    try:
        sha = sh(["git", "rev-parse", "--short", "HEAD"]).strip()
    except Exception:
        sha = "?"
    print("=== 1994 Old School playtest progress ===")
    print(datetime.datetime.now().strftime("%Y-%m-%d %H:%M"), " | integration", sha)
    print()

    union = {}  # card_id -> closed(bool); dedups cards shared across decks
    if not decks:
        print("  (no deck-tracking issues matched — check titles via `mb list`)")
    for d in decks:
        refs = {r for r in ref_re.findall(d.get("description", "") or "") if r != d["id"]}
        done = total = 0
        for c in sorted(refs):
            ci = by_id.get(c)
            if ci is None:
                continue  # reference to something that isn't a tracked issue
            total += 1
            closed = ci.get("status") == "closed"
            if closed:
                done += 1
            union[c] = closed
        pct = (done * 100 // total) if total else 0
        print(f"  {clean(d['title']):42.42s} {done:2d}/{total:<3d} ({pct:3d}%)  [{d['id']}]")

    u_total = len(union)
    u_done = sum(1 for v in union.values() if v)
    cc = [i for i in issues if i.get("title", "").startswith("Card Compatibility:")]
    cc_done = sum(1 for i in cc if i.get("status") == "closed")
    print()
    print(f"  {'UNIQUE deck-linked cards (union)':42s} {u_done:2d}/{u_total:<3d} "
          f"({(u_done * 100 // u_total) if u_total else 0:3d}%)")
    print(f"  {'ALL Card Compatibility: issues':42s} {cc_done:2d}/{len(cc):<3d} "
          f"({(cc_done * 100 // len(cc)) if cc else 0:3d}%)")


if __name__ == "__main__":
    main()
