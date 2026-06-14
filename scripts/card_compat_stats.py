#!/usr/bin/env python3
"""Reusable card-compatibility statistics for the DeepScry minibeads DB.

This ONE durable script subsumes the old per-collection progress scripts
(``scripts/temp/oldschool_progress.py`` and
``scripts/temp/championship_2025_progress.py``). It answers, from the
version-controlled minibeads issue store, three questions about every
tracked Magic card:

    * how many cards have a tracking issue at all,
    * what fraction of those are CLOSED (reached WORKING), and
    * what fraction are PUZZLE-BACKED (carry a regression puzzle).

A "card-tracking issue" is a minibeads issue whose title begins with
``Card Compatibility:`` -- the convention both old scripts relied on and
the one the compatibility-tracking workflow files under. The card name is
the title remainder.

The unit of counting is the ISSUE, not the distinct card name: a handful
of cards currently carry two ``Card Compatibility:`` issues (e.g. one per
printing/deck), and each is counted, so the global denominator matches the
old scripts' "ALL Card Compatibility: issues" line exactly. (De-duplicating
those issues is a beads-hygiene task, not this report's job.)

Year/Set breakdown
------------------
Each card issue's description carries a ``Set: <CODE>`` line. The set CODE
is resolved to a release YEAR using the canonical ``editions/`` data
(``Code=`` / ``Date=`` in each edition file's ``[metadata]`` block -- the
same source ``mtg-engine/src/loader/edition.rs`` parses). Cards whose set
code is absent or not present in ``editions/`` are bucketed under the
``unknown`` year / set so the totals always reconcile.

Puzzle-backed
-------------
A card is counted PUZZLE-BACKED if its issue carries the ``puzzle-tested``
label OR a ``PUZZLE_FILE:`` line in its description. The per-card puzzle
auditor that will stamp those markers is a later increment (mtg-948 Part
B); today essentially zero cards are puzzle-backed, which is reported
faithfully (0% is a valid, expected result, not an error).

Deck collections
----------------
``--deck-collection <name>`` restricts every statistic to the cards that
appear in the decks of one website deck-collection. The valid names are
exactly the collections the web launchers expose (see ``DECK_COLLECTIONS``
in ``web/launcher.html`` / ``web/solo_launcher.html`` and the export-wasm
deck-glob defaults in ``mtg-engine/src/main.rs``). They are derived here
from the on-disk ``decks/`` tree so the script and the site never drift.
Run ``--help`` to see the live list.

This module treats the beads issues as STRUCTURED data: it consumes
``mb list --json`` and queries typed fields, never regex-scraping the
human-readable issue listing. The only line-oriented parsing is of the
genuinely line-structured ``key=value`` edition / ``.dck`` files and the
single ``Set:`` description line, each split on its delimiter rather than
substring-matched.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------
# Repo layout discovery
# --------------------------------------------------------------------------

CARD_ISSUE_TITLE_PREFIX = "Card Compatibility: "
PUZZLE_LABEL = "puzzle-tested"
PUZZLE_MARKER = "PUZZLE_FILE:"
UNKNOWN = "unknown"


def repo_root() -> Path:
    """Locate the project root (the dir holding ``editions/`` and ``decks/``).

    Falls back to ``git rev-parse`` and finally to this file's grandparent so
    the script works when invoked from anywhere inside a worktree.
    """
    here = Path(__file__).resolve()
    # scripts/card_compat_stats.py -> repo root is parent of scripts/
    candidate = here.parent.parent
    if (candidate / "editions").is_dir() and (candidate / "decks").is_dir():
        return candidate
    try:
        out = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        if out:
            return Path(out)
    except Exception:
        pass
    return candidate


# --------------------------------------------------------------------------
# Edition (set -> year) data
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Edition:
    code: str
    name: str
    year: str  # 4-digit string, or UNKNOWN


def load_editions(editions_dir: Path) -> dict[str, Edition]:
    """Parse every ``editions/*.txt`` ``[metadata]`` block into Code->Edition.

    Each edition file starts with a ``[metadata]`` section of ``Key=Value``
    lines; we read ``Code``, ``Name`` and ``Date`` (year = first 4 chars),
    stopping at the ``[cards]`` section. This is line-structured key=value
    parsing -- split on the first ``=`` -- not substring matching.
    """
    editions: dict[str, Edition] = {}
    if not editions_dir.is_dir():
        return editions
    for path in sorted(editions_dir.glob("*.txt")):
        code = name = date = None
        try:
            with path.open(encoding="utf-8", errors="replace") as fh:
                for raw in fh:
                    line = raw.strip()
                    if line == "[cards]":
                        break
                    if "=" not in line:
                        continue
                    key, _, value = line.partition("=")
                    key = key.strip()
                    value = value.strip()
                    if key == "Code":
                        code = value
                    elif key == "Name":
                        name = value
                    elif key == "Date":
                        date = value
        except OSError:
            continue
        if code:
            year = date[:4] if (date and len(date) >= 4 and date[:4].isdigit()) else UNKNOWN
            editions[code] = Edition(code=code, name=name or code, year=year)
    return editions


# --------------------------------------------------------------------------
# Beads issue ingestion
# --------------------------------------------------------------------------


@dataclass
class CardIssue:
    id: str
    name: str          # card name (title remainder)
    closed: bool
    puzzle_backed: bool
    set_code: str      # raw Set: code, or UNKNOWN
    year: str          # resolved release year, or UNKNOWN
    set_name: str      # human set name, or the raw code / UNKNOWN


def load_card_issues(beads_dir: Path, editions: dict[str, Edition]) -> list[CardIssue]:
    """Return one CardIssue per ``Card Compatibility:`` minibeads issue.

    Consumes ``mb list --json`` (structured) and queries typed fields. The
    only text parse is the single ``Set: <code>`` description line, split on
    whitespace after the ``Set:`` key.
    """
    try:
        proc = subprocess.run(
            ["mb", "--mb-beads-dir", str(beads_dir), "list", "--json", "--limit", "1000000"],
            capture_output=True, text=True, check=True,
        )
    except FileNotFoundError:
        sys.exit("error: `mb` (minibeads) is not on PATH")
    except subprocess.CalledProcessError as exc:
        sys.exit(f"error: `mb list --json` failed: {exc.stderr.strip()}")

    try:
        raw_issues = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        sys.exit(f"error: could not parse `mb list --json` output: {exc}")
    if not raw_issues:
        sys.exit("error: `mb` returned no issues (run from inside the repo / its .beads).")

    # The leading token of a `Set:` line is the canonical set code. The store's
    # `Set:` lines are written inconsistently -- the code may be followed by a
    # parenthetical gloss `ARN (Arabian Nights)`, a `/` qualifier
    # `ATLA / 2025 Standard`, `4ED/LEA`, `Classic/Khans of Tarkir`, or a
    # trailing `.` `LEA. Deck: ...`. We tokenize: capture the first run of
    # code characters (letters/digits) after `Set:`, which yields the bare
    # primary code (`ATLA`, `4ED`, `Classic`, `LEA`, ...) by stopping at the
    # first delimiter -- structured tokenization, not substring matching.
    set_line_re = re.compile(r"^\s*Set:\s*(?P<code>[A-Za-z0-9]+)", re.MULTILINE)
    cards: list[CardIssue] = []
    for issue in raw_issues:
        title = issue.get("title", "") or ""
        if not title.startswith(CARD_ISSUE_TITLE_PREFIX):
            continue
        name = title[len(CARD_ISSUE_TITLE_PREFIX):].strip()
        desc = issue.get("description", "") or ""
        labels = issue.get("labels") or []

        match = set_line_re.search(desc)
        # The Set: code may be followed by "(mtg-NNN)"; the \S+ above grabs the
        # bare token, which never includes the parenthesised issue ref because
        # of the intervening space.
        set_code = match.group("code") if match else UNKNOWN
        edition = editions.get(set_code)
        if edition is not None:
            year = edition.year
            set_name = edition.name
        else:
            year = UNKNOWN
            set_name = set_code  # keep the raw code so unresolved sets stay visible

        cards.append(CardIssue(
            id=issue.get("id", "?"),
            name=name,
            closed=(issue.get("status") == "closed"),
            puzzle_backed=(PUZZLE_LABEL in labels) or (PUZZLE_MARKER in desc),
            set_code=set_code,
            year=year,
            set_name=set_name,
        ))
    return cards


# --------------------------------------------------------------------------
# Deck collections (mirrors the website's options, derived from decks/)
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Collection:
    """A website deck-collection: a name plus the deck-file globs feeding it."""
    key: str
    label: str
    globs: tuple[str, ...]


def discover_collections(root: Path) -> dict[str, Collection]:
    """Build the deck-collection table that mirrors the web launchers.

    The fixed, non-championship collections map to disk directories exactly
    as ``mtg-engine/src/main.rs`` (the export-wasm deck-glob defaults) and the
    ``DECK_COLLECTIONS`` filters in ``web/launcher.html`` do. Each World
    Championship year present under ``decks/championship/<year>/`` becomes its
    own ``championship_<year>`` collection, matching the per-year entries the
    launchers add programmatically.
    """
    collections: dict[str, Collection] = {}

    def add(key: str, label: str, *globs: str) -> None:
        # Only advertise a collection if at least one of its globs actually
        # matches a deck file on disk, so the --help list never names a dead
        # collection.
        if any(next(root.glob(g), None) is not None for g in globs):
            collections[key] = Collection(key=key, label=label, globs=globs)

    # Old School draws from several on-disk dirs (the web filter's surname/
    # archetype substrings all live under these trees).
    add("old_school", "Old School 1994",
        "decks/old_school/**/*.dck", "decks/old_school2/**/*.dck",
        "decks/rn_os/**/*.dck")
    add("booster_draft", "Booster Draft",
        "decks/booster_draft/**/*.dck", "decks/avatar/**/*.dck",
        "decks/tmnt/**/*.dck")
    add("commander", "Commander", "decks/commander/**/*.dck")

    champ_root = root / "decks" / "championship"
    if champ_root.is_dir():
        for year_dir in sorted(p for p in champ_root.iterdir() if p.is_dir()):
            year = year_dir.name
            if not any(year_dir.glob("**/*.dck")):
                continue
            collections[f"championship_{year}"] = Collection(
                key=f"championship_{year}",
                label=f"{year} World Championship",
                globs=(f"decks/championship/{year}/**/*.dck",),
            )
    return collections


def cards_in_collection(root: Path, collection: Collection) -> set[str]:
    """Return the set of distinct card names across a collection's decks.

    ``.dck`` files are INI-like: a ``[Main]`` / ``[Sideboard]`` section of
    ``<count> <card name>`` lines. We split each line once on whitespace to
    separate the leading integer count from the card name -- structured
    line parsing, not substring matching. Both main deck and sideboard
    count, matching what the per-collection scripts treated as "deck-linked".
    """
    names: set[str] = set()
    for glob in collection.globs:
        for dck in sorted(root.glob(glob)):
            names.update(_card_names_in_dck(dck))
    return names


def _card_names_in_dck(path: Path) -> set[str]:
    names: set[str] = set()
    section: str | None = None
    try:
        with path.open(encoding="utf-8", errors="replace") as fh:
            for raw in fh:
                line = raw.strip()
                if not line:
                    continue
                if line.startswith("[") and line.endswith("]"):
                    section = line[1:-1].strip().lower()
                    continue
                if section in ("main", "sideboard"):
                    count, _, name = line.partition(" ")
                    if count.isdigit() and name.strip():
                        names.add(name.strip())
    except OSError:
        pass
    return names


# --------------------------------------------------------------------------
# Aggregation
# --------------------------------------------------------------------------


@dataclass
class Bucket:
    total: int = 0
    closed: int = 0
    puzzle_backed: int = 0

    def add(self, card: CardIssue) -> None:
        self.total += 1
        if card.closed:
            self.closed += 1
        if card.puzzle_backed:
            self.puzzle_backed += 1

    @property
    def pct_closed(self) -> int:
        return (self.closed * 100 // self.total) if self.total else 0

    @property
    def pct_puzzle(self) -> int:
        return (self.puzzle_backed * 100 // self.total) if self.total else 0


@dataclass
class YearGroup:
    year: str
    overall: Bucket = field(default_factory=Bucket)
    sets: dict[str, Bucket] = field(default_factory=dict)
    set_names: dict[str, str] = field(default_factory=dict)


def aggregate(cards: list[CardIssue]) -> tuple[Bucket, dict[str, YearGroup]]:
    overall = Bucket()
    years: dict[str, YearGroup] = {}
    for card in cards:
        overall.add(card)
        yg = years.setdefault(card.year, YearGroup(year=card.year))
        yg.overall.add(card)
        sb = yg.sets.setdefault(card.set_code, Bucket())
        yg.set_names[card.set_code] = card.set_name
        sb.add(card)
    return overall, years


def _year_sort_key(year: str) -> tuple[int, str]:
    # Numeric years ascending; UNKNOWN last.
    return (0, f"{int(year):04d}") if year.isdigit() else (1, year)


# --------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------


def render_text(overall: Bucket, years: dict[str, YearGroup], scope: str) -> str:
    lines: list[str] = []
    lines.append(f"=== Card-compatibility stats ({scope}) ===")
    lines.append("")
    lines.append(
        f"  cards tracked : {overall.total}")
    lines.append(
        f"  closed        : {overall.closed}/{overall.total} ({overall.pct_closed}%)")
    lines.append(
        f"  puzzle-backed : {overall.puzzle_backed}/{overall.total} ({overall.pct_puzzle}%)")
    lines.append("")
    if not overall.total:
        lines.append("  (no card-tracking issues in scope)")
        return "\n".join(lines)

    header = f"  {'YEAR / Set':32s} {'closed':>13s} {'puzzle':>13s}"
    lines.append(header)
    lines.append("  " + "-" * (len(header) - 2))
    for year in sorted(years, key=_year_sort_key):
        yg = years[year]
        b = yg.overall
        lines.append(
            f"  {year:32s} "
            f"{b.closed:>4d}/{b.total:<4d}({b.pct_closed:>3d}%) "
            f"{b.puzzle_backed:>4d}/{b.total:<4d}({b.pct_puzzle:>3d}%)")
        for set_code in sorted(yg.sets, key=lambda c: yg.set_names.get(c, c).lower()):
            sb = yg.sets[set_code]
            set_label = yg.set_names.get(set_code, set_code)
            display = f"{set_label} ({set_code})" if set_label != set_code else set_code
            lines.append(
                f"    {display:30s} "
                f"{sb.closed:>4d}/{sb.total:<4d}({sb.pct_closed:>3d}%) "
                f"{sb.puzzle_backed:>4d}/{sb.total:<4d}({sb.pct_puzzle:>3d}%)")
    return "\n".join(lines)


def render_json(overall: Bucket, years: dict[str, YearGroup], scope: str) -> str:
    payload = {
        "scope": scope,
        "overall": {
            "cards_tracked": overall.total,
            "closed": overall.closed,
            "pct_closed": overall.pct_closed,
            "puzzle_backed": overall.puzzle_backed,
            "pct_puzzle_backed": overall.pct_puzzle,
        },
        "years": [],
    }
    for year in sorted(years, key=_year_sort_key):
        yg = years[year]
        b = yg.overall
        year_entry = {
            "year": year,
            "cards_tracked": b.total,
            "closed": b.closed,
            "pct_closed": b.pct_closed,
            "puzzle_backed": b.puzzle_backed,
            "pct_puzzle_backed": b.pct_puzzle,
            "sets": [],
        }
        for set_code in sorted(yg.sets, key=lambda c: yg.set_names.get(c, c).lower()):
            sb = yg.sets[set_code]
            year_entry["sets"].append({
                "set_code": set_code,
                "set_name": yg.set_names.get(set_code, set_code),
                "cards_tracked": sb.total,
                "closed": sb.closed,
                "pct_closed": sb.pct_closed,
                "puzzle_backed": sb.puzzle_backed,
                "pct_puzzle_backed": sb.pct_puzzle,
            })
        payload["years"].append(year_entry)
    return json.dumps(payload, indent=2)


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def build_parser(collection_keys: list[str]) -> argparse.ArgumentParser:
    valid = ", ".join(collection_keys) if collection_keys else "(none discovered)"
    parser = argparse.ArgumentParser(
        prog="card_compat_stats.py",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description=(
            "Report DeepScry card-compatibility progress from the minibeads "
            "issue store: how many cards are tracked, what percent are closed, "
            "and what percent are puzzle-backed -- globally or scoped to a "
            "website deck-collection, broken down by release year and set."),
        epilog="Valid --deck-collection names: " + valid,
    )
    parser.add_argument(
        "--deck-collection", metavar="NAME",
        help=("restrict stats to the cards in this website deck-collection. "
              "Valid names: " + valid))
    parser.add_argument(
        "--year", metavar="YYYY",
        help="restrict stats to a single release year (e.g. 1994, or 'unknown')")
    parser.add_argument(
        "--set", metavar="CODE", dest="set_code",
        help="restrict stats to a single set code (e.g. LEA, ATQ)")
    parser.add_argument(
        "--json", action="store_true",
        help="emit machine-readable JSON instead of the columnar text report")
    return parser


def main(argv: list[str] | None = None) -> int:
    root = repo_root()
    editions = load_editions(root / "editions")
    collections = discover_collections(root)
    collection_keys = list(collections.keys())

    parser = build_parser(collection_keys)
    args = parser.parse_args(argv)

    cards = load_card_issues(root / ".beads", editions)
    scope_parts: list[str] = []

    if args.deck_collection is not None:
        coll = collections.get(args.deck_collection)
        if coll is None:
            parser.error(
                f"unknown --deck-collection '{args.deck_collection}'. "
                f"Valid names: {', '.join(collection_keys) or '(none)'}")
        wanted = cards_in_collection(root, coll)
        cards = [c for c in cards if c.name in wanted]
        scope_parts.append(f"collection={coll.key}")

    if args.year is not None:
        cards = [c for c in cards if c.year == args.year]
        scope_parts.append(f"year={args.year}")

    if args.set_code is not None:
        cards = [c for c in cards if c.set_code == args.set_code]
        scope_parts.append(f"set={args.set_code}")

    scope = ", ".join(scope_parts) if scope_parts else "global"
    overall, years = aggregate(cards)

    if args.json:
        print(render_json(overall, years, scope))
    else:
        print(render_text(overall, years, scope))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
