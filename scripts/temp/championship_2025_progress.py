#!/usr/bin/env python3
"""Quantify 2025 World Championship deck playtest progress (transient reporting tool).

For the 2025 championship deck tracking issue (mtg-881) and its sibling/child deck issues,
parse the card issue IDs referenced in its description and report how many are CLOSED.

Usage:  scripts/temp/championship_2025_progress.py
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
        sys.exit("error: mb returned no issues.")

    by_id = {i["id"]: i for i in issues}
    by_title = {i["title"]: i for i in issues}

    # 2025 Championship deck tracking issue
    target_tracker = "mtg-881"

    if target_tracker not in by_id:
        sys.exit(f"error: {target_tracker} not found in beads database")

    tracker_issue = by_id[target_tracker]

    try:
        sha = sh(["git", "rev-parse", "--short", "HEAD"]).strip()
    except Exception:
        sha = "?"

    print("=== 2025 World Championship Playtest Progress ===")
    print(datetime.datetime.now().strftime("%Y-%m-%d %H:%M"), " | integration", sha)
    print()

    # Parse the card names from mtg-881's markdown table
    card_names = []
    in_table = False
    for line in (tracker_issue.get("description", "") or "").splitlines():
        if "| Card | Type |" in line:
            in_table = True
            continue
        if in_table:
            if not line.strip().startswith("|"):
                in_table = False
                continue
            if line.strip().startswith("|-"):
                continue
            parts = [p.strip() for p in line.split("|")]
            if len(parts) >= 3:
                card_name = parts[1]
                if card_name and not card_name.startswith("-") and card_name != "Card":
                    card_names.append(card_name)

    # Track 2025 deck compatibility issues
    # e.g., mtg-874 "Deck Compatibility: 04 Henry Temur Otters (2025 World Championship)"
    def is_2025_deck(i):
        title = i.get("title", "")
        return i["id"] == target_tracker or (title.startswith("Deck Compatibility:") and "2025" in title)

    def clean(title):
        t = re.sub(r"^TRACK: ", "", title)
        t = re.sub(r"^Deck Compatibility: ", "", t)
        t = re.sub(r" — full deck compatibility$", "", t)
        t = re.sub(r" — full compatibility$", "", t)
        return t.strip().strip("'")

    decks = sorted((i for i in issues if is_2025_deck(i)), key=lambda i: clean(i["title"]))

    ref_re = re.compile(r"mtg-[0-9a-z]+")
    for d in decks:
        refs = {r for r in ref_re.findall(d.get("description", "") or "") if r != d["id"]}
        done = total = 0
        for c in sorted(refs):
            ci = by_id.get(c)
            if ci is None:
                continue
            if is_2025_deck(ci):
                continue
            total += 1
            closed = ci.get("status") == "closed"
            if closed:
                done += 1
        pct = (done * 100 // total) if total else 0
        print(f"  {clean(d['title']):42.42s} {done:2d}/{total:<3d} ({pct:3d}%)  [{d['id']}]")

    # Now show the unique card compatibility progress from mtg-881's list
    card_done = 0
    card_total = 0
    unresolved_cards = []
    
    for name in sorted(card_names):
        title = f"Card Compatibility: {name}"
        card_total += 1
        ci = by_title.get(title)
        if ci is not None and ci.get("status") == "closed":
            card_done += 1
        else:
            issue_id = ci["id"] if ci else "unfiled"
            unresolved_cards.append((name, issue_id))

    card_pct = (card_done * 100 // card_total) if card_total else 0
    print()
    print(f"  {'UNIQUE deck-linked cards (union)':42s} {card_done:2d}/{card_total:<3d} ({card_pct:3d}%)")
    
    # Optional details
    print()
    print("  === Unresolved Cards ===")
    for name, iid in unresolved_cards[:10]:
        print(f"    - {name:30s} [{iid}]")
    if len(unresolved_cards) > 10:
        print(f"    ... and {len(unresolved_cards) - 10} more.")


if __name__ == "__main__":
    main()
