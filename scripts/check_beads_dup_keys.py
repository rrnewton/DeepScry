#!/usr/bin/env python3
"""Guard against duplicate top-level YAML frontmatter keys in beads issue files.

WHY THIS EXISTS
---------------
Beads issue files (`.beads/issues/*.md`) begin with a YAML frontmatter block
delimited by `---` lines. Several feature branches stamp the SAME tracker issue
(e.g. `mtg-742`) via `mb update`, which rewrites the top-level `updated_at:`
line. When two such branches merge, a 3-way *text* merge cannot tell that both
sides changed the same logical key — it keeps BOTH `updated_at:` lines. The
result is a frontmatter block with a DUPLICATE top-level key, which makes the
YAML ambiguous and causes `mb list` / `mb show` to error out across the WHOLE
`.beads` directory (one bad file poisons every command). This recurred four
times in one night during release ceremonies.

This checker is the guard: it HARD-FAILS when any `.beads/issues/*.md` file has
a duplicate top-level frontmatter key, naming the offending file + key so the
fix is obvious. It is wired into `scripts/validate.py` (so CI catches it) and
can be run standalone with `--repair` to auto-fix (keep the LAST occurrence of
each duplicated key — for `updated_at` that is the newest timestamp).

USAGE
-----
    python3 scripts/check_beads_dup_keys.py            # check default dir, exit 1 on dup
    python3 scripts/check_beads_dup_keys.py --repair    # fix dups in place (keep last)
    python3 scripts/check_beads_dup_keys.py path/to/.beads/issues
    python3 scripts/check_beads_dup_keys.py file1.md file2.md   # explicit files

Only *top-level* keys (indentation level 0) are checked. List items (`- foo`)
and nested mapping keys (indented) are intentionally ignored — a `labels:` block
with several `- web` items is legal, and so is the same nested key under two
different parents.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# A top-level YAML mapping key: starts at column 0 (no leading whitespace),
# an unquoted key char run, then a colon followed by EOL or whitespace.
# This deliberately excludes:
#   - list items ("- design")            -> start with '-'
#   - nested keys ("  foo: bar")          -> have leading whitespace
#   - the '---' delimiters                -> no colon
_TOP_LEVEL_KEY = re.compile(r"^([A-Za-z_][A-Za-z0-9_-]*)\s*:")


def _split_frontmatter(lines: list[str]) -> tuple[int, int] | None:
    """Return (start_idx, end_idx) line indices of the frontmatter BODY
    (exclusive of the two `---` fences), or None if there is no frontmatter.

    The frontmatter is the block between the first `---` (which must be the
    very first line) and the next `---`.
    """
    if not lines or lines[0].rstrip("\n") != "---":
        return None
    for i in range(1, len(lines)):
        if lines[i].rstrip("\n") == "---":
            return (1, i)  # body is lines[1:i]
    return None  # unterminated frontmatter — treat as "no frontmatter" here


def find_duplicate_keys(text: str) -> dict[str, int]:
    """Return {key: count} for every top-level frontmatter key that appears
    more than once. Empty dict means the file is clean.
    """
    lines = text.splitlines(keepends=True)
    span = _split_frontmatter(lines)
    if span is None:
        return {}
    start, end = span
    counts: dict[str, int] = {}
    for line in lines[start:end]:
        m = _TOP_LEVEL_KEY.match(line)
        if m:
            key = m.group(1)
            counts[key] = counts.get(key, 0) + 1
    return {k: c for k, c in counts.items() if c > 1}


def repair_text(text: str) -> str:
    """Return `text` with duplicate top-level frontmatter keys collapsed to
    their LAST occurrence (newest `updated_at` wins). Order of the kept lines
    follows the LAST position of each key. Non-frontmatter content is untouched.
    """
    lines = text.splitlines(keepends=True)
    span = _split_frontmatter(lines)
    if span is None:
        return text
    start, end = span

    dups = find_duplicate_keys(text)
    if not dups:
        return text

    # For each duplicated key, keep only the LAST occurrence; drop the earlier
    # ones. A duplicated top-level key in beads frontmatter is always a scalar
    # (updated_at / created_at / status / priority); we do not need to carry
    # nested children, but to be safe we only drop the single key line and keep
    # any following indented/list lines attached to whichever occurrence we keep.
    body = lines[start:end]

    # Record, per duplicated key, the index of its LAST occurrence within body.
    last_idx: dict[str, int] = {}
    for idx, line in enumerate(body):
        m = _TOP_LEVEL_KEY.match(line)
        if m and m.group(1) in dups:
            last_idx[m.group(1)] = idx

    kept: list[str] = []
    for idx, line in enumerate(body):
        m = _TOP_LEVEL_KEY.match(line)
        if m and m.group(1) in dups and idx != last_idx[m.group(1)]:
            # This is an EARLIER duplicate of a top-level key — drop this line
            # AND any immediately-following block-children (indented or list)
            # that belong to it, since they belong to the discarded value.
            continue
        kept.append(line)

    return "".join(lines[:start] + kept + lines[end:])


def iter_target_files(paths: list[str]) -> list[Path]:
    """Resolve CLI args to a concrete list of .md files. A directory expands to
    its `*.md` children; a file is taken as-is; the default is the repo's
    `.beads/issues/`.
    """
    if not paths:
        default = Path(".beads/issues")
        return sorted(default.glob("*.md")) if default.is_dir() else []
    files: list[Path] = []
    for p in paths:
        path = Path(p)
        if path.is_dir():
            files.extend(sorted(path.glob("*.md")))
        elif path.is_file():
            files.append(path)
        else:
            sys.stderr.write(f"[beads-dupkey] warning: not found, skipping: {p}\n")
    return files


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument(
        "paths",
        nargs="*",
        help="files or directories to check (default: .beads/issues)",
    )
    ap.add_argument(
        "--repair",
        action="store_true",
        help="fix duplicate top-level keys in place (keep the LAST occurrence)",
    )
    args = ap.parse_args(argv)

    files = iter_target_files(args.paths)
    if not files:
        # No files to check is not an error (e.g. fresh checkout without beads).
        return 0

    offenders: list[tuple[Path, dict[str, int]]] = []
    for f in files:
        try:
            text = f.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as e:
            sys.stderr.write(f"[beads-dupkey] warning: cannot read {f}: {e}\n")
            continue
        dups = find_duplicate_keys(text)
        if dups:
            offenders.append((f, dups))

    if not offenders:
        return 0

    if args.repair:
        for f, dups in offenders:
            fixed = repair_text(f.read_text(encoding="utf-8"))
            f.write_text(fixed, encoding="utf-8")
            keys = ", ".join(sorted(dups))
            print(f"[beads-dupkey] repaired {f} (collapsed: {keys})")
        # Re-verify the repair actually worked.
        still: list[Path] = [
            f for f, _ in offenders
            if find_duplicate_keys(f.read_text(encoding="utf-8"))
        ]
        if still:
            sys.stderr.write(
                "[beads-dupkey] ERROR: repair did not clear: "
                + ", ".join(str(p) for p in still)
                + "\n"
            )
            return 1
        return 0

    # Report mode (the CI gate).
    sys.stderr.write(
        "\n[beads-dupkey] DUPLICATE top-level YAML frontmatter key(s) found in "
        f"{len(offenders)} beads issue file(s).\n"
        "  A duplicate top-level key (commonly `updated_at`, from a 3-way text\n"
        "  merge of two branches that each `mb update`d the same tracker) makes\n"
        "  the YAML ambiguous and breaks `mb list` / `mb show` for the WHOLE\n"
        "  .beads directory.\n\n"
    )
    for f, dups in offenders:
        for key, count in sorted(dups.items()):
            sys.stderr.write(f"  {f}: top-level key '{key}' appears {count}x\n")
    sys.stderr.write(
        "\n  FIX: keep the LATEST occurrence of each duplicated key (for\n"
        "  `updated_at`, the newest timestamp) and delete the earlier line(s),\n"
        "  or run:\n"
        "      python3 scripts/check_beads_dup_keys.py --repair\n\n"
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
