#!/usr/bin/env python3
"""Suite flakiness dashboard (mtg-593).

Reads experiment_results/flakiness_db.csv and summarizes the flakiness state of
the validation suite: counts by classification, top flakers, and real-bug rows.
This is the at-a-glance answer to "is validate's redness real?".

A run whose only non-passing rows are `timeout-under-load` + already-tracked
`known-desync` is GREEN-MODULO-KNOWN-ISSUES. Any `true-nondeterministic` row is
an unexplained flake that needs investigation.

Usage:
    flakiness_report.py                 # latest measurement per test
    flakiness_report.py --all           # every recorded row
    flakiness_report.py --class known-desync   # filter to one classification

See ai_docs/reference/TEST_FLAKINESS.md.
"""
import argparse
import csv
import datetime
import subprocess
from pathlib import Path

GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
RED = "\033[0;31m"
CYAN = "\033[0;36m"
NC = "\033[0m"

CLASS_ORDER = [
    "deterministic-pass",
    "timeout-under-load",
    "known-desync",
    "true-nondeterministic",
]
CLASS_COLOR = {
    "deterministic-pass": GREEN,
    "timeout-under-load": YELLOW,
    "known-desync": CYAN,
    "true-nondeterministic": RED,
}


def repo_root():
    p = Path(__file__).resolve().parent
    while p != p.parent:
        if (p / "Cargo.toml").exists() and (p / "Makefile").exists():
            return p
        p = p.parent
    return Path.cwd()


ROOT = repo_root()
DB = ROOT / "experiment_results" / "flakiness_db.csv"


def load():
    if not DB.exists():
        raise SystemExit(f"no flakiness DB at {DB}")
    with DB.open(newline="") as f:
        return list(csv.DictReader(f))


def latest_per_test(rows):
    """Keep the newest row per canonical_name (by timestamp)."""
    best = {}
    for r in rows:
        n = r["canonical_name"]
        if n not in best or r["timestamp"] > best[n]["timestamp"]:
            best[n] = r
    return list(best.values())


def short_sha():
    try:
        return subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT,
                              capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return "?"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true", help="every row, not just latest")
    ap.add_argument("--class", dest="cls", default=None, help="filter classification")
    args = ap.parse_args()

    rows = load()
    rows = rows if args.all else latest_per_test(rows)
    if args.cls:
        rows = [r for r in rows if r["classification"] == args.cls]

    print(f"{CYAN}=== Suite flakiness ({'all rows' if args.all else 'latest per test'}) ==={NC}")
    print(datetime.datetime.now().strftime("%Y-%m-%d %H:%M"), "| integration", short_sha())
    print()

    by_class = {}
    for r in rows:
        by_class.setdefault(r["classification"], []).append(r)

    total = len(rows)
    flaky = sum(len(v) for k, v in by_class.items() if k != "deterministic-pass")
    print(f"tests tracked: {total}    non-clean: {flaky}    "
          f"clean: {total - flaky}")
    print()
    print("by classification:")
    for cls in CLASS_ORDER + [c for c in by_class if c not in CLASS_ORDER]:
        if cls in by_class:
            c = CLASS_COLOR.get(cls, NC)
            print(f"  {c}{cls:<24}{NC} {len(by_class[cls])}")
    print()

    # Top flakers (highest flakiness_pct, non-clean).
    flakers = sorted(
        (r for r in rows if r["classification"] != "deterministic-pass"),
        key=lambda r: float(r["flakiness_pct"] or 0), reverse=True,
    )
    if flakers:
        print(f"{CYAN}top flakers:{NC}")
        print(f"  {'flaky%':>7}  {'class':<22} {'issue':<10} name")
        for r in flakers[:20]:
            c = CLASS_COLOR.get(r["classification"], NC)
            print(f"  {r['flakiness_pct']:>7}  {c}{r['classification']:<22}{NC} "
                  f"{r['issue'] or '-':<10} {r['canonical_name']}")
        print()

    real = by_class.get("true-nondeterministic", [])
    if real:
        print(f"{RED}UNEXPLAINED (true-nondeterministic) -- needs investigation:{NC}")
        for r in real:
            print(f"  {r['canonical_name']} ({r['flakiness_pct']}%)")
    else:
        print(f"{GREEN}No unexplained true-nondeterministic flakes.{NC} "
              f"(remaining non-clean rows are env-timeout or tracked bugs.)")


if __name__ == "__main__":
    main()
