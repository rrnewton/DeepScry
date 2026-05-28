#!/usr/bin/env python3
"""Global test-flakiness stress harness (mtg-j010x).

Runs a single CANONICAL test name in ISOLATION N times with BOUNDED
concurrency, records pass/fail/timeout per run, and computes a flakiness rate +
auto-classification. Also supports a bounded "stress-all" sweep.

See ai_docs/reference/TEST_FLAKINESS.md for the canonical-name scheme, the DB
schema, and the classification rule. The name -> command mapping below
(KIND_RUNNERS) is the authoritative decoder for canonical names.

Usage:
    flakiness_stress.py one   <canonical_name> [--runs N] [--concurrency K] ...
    flakiness_stress.py stress-all            [--runs N] [--concurrency K] ...
    flakiness_stress.py list

Examples:
    flakiness_stress.py one validate.shell_script_tests.commander_e2e --runs 20 --record
    flakiness_stress.py stress-all --runs 3 --concurrency 4 --record
    flakiness_stress.py one validate.mtg-engine--determinism_e2e.determinism_holds \
        --runs 50 --concurrency 1 --record

CPU COURTESY: keep --concurrency bounded. Oversubscribing manufactures the
timeout-under-load false-flakes this tool exists to distinguish. For a serious
single-test sweep use --concurrency 1 on an idle box.
"""
import argparse
import concurrent.futures
import csv
import datetime
import json
import os
import re
import subprocess
import sys
from pathlib import Path

RED = "\033[0;31m"
GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
CYAN = "\033[0;36m"
NC = "\033[0m"

DB_PATH_REL = "experiment_results/flakiness_db.csv"
DB_HEADER = [
    "timestamp", "git_commit", "git_depth", "cpu", "canonical_name", "kind",
    "runs", "fails", "timeouts", "flakiness_pct", "classification",
    "issue", "concurrency", "notes",
]

# Outcome of a single run.
PASS, FAIL, TIMEOUT = "pass", "fail", "timeout"


def cprint(color, msg):
    print(f"{color}{msg}{NC}")


def repo_root():
    """Workspace root = dir containing Makefile + Cargo.toml."""
    p = Path(__file__).resolve().parent
    while p != p.parent:
        if (p / "Cargo.toml").exists() and (p / "Makefile").exists():
            return p
        p = p.parent
    return Path.cwd()


ROOT = repo_root()


def git_sha():
    try:
        return subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT,
                              capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return "unknown"


def git_depth():
    try:
        return subprocess.run(["git", "rev-list", "--count", "HEAD"], cwd=ROOT,
                              capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return "0"


def cpu_name():
    """CPU identifier matching the benchmark dir convention (see
    scripts/run_benchmark.sh get_cpu_name): /proc/cpuinfo model name, spaces
    -> '_', strip anything but [A-Za-z0-9_-]. Flakiness is CPU-/core-count-
    sensitive (timeout-under-load), so every measurement records its host."""
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    raw = line.split(":", 1)[1].strip()
                    return re.sub(r"[^A-Za-z0-9_-]", "", raw.replace(" ", "_"))
    except Exception:
        pass
    return "unknown_cpu"


# ---------------------------------------------------------------------------
# Canonical-name -> command decoders.
#
# Each entry maps a `validate.<kind>...` prefix to a function that, given the
# remainder of the dotted name, returns (argv, kind). argv is run from ROOT.
# ---------------------------------------------------------------------------

def _cargo_cmd(rest):
    """validate.<pkg>--<binary>.<module::test>  OR  validate.<pkg>--lib.<path>.

    Decodes to a `cargo test -p <pkg> --test <binary> -- --exact <path>` run.
    """
    # rest = "<pkg>--<binary>.<module::test::path>"
    binary_part, sep, test_path = rest.partition(".")
    pkg, _, binary = binary_part.partition("--")
    cmd = ["cargo", "test", "-p", pkg]
    if binary == "lib":
        cmd += ["--lib"]
    else:
        cmd += ["--test", binary]
    cmd += ["--features", "network", "--"]
    if test_path:
        cmd += ["--exact", test_path]
    cmd += ["--nocapture", "--test-threads=1"]
    return cmd, "cargo"


def _shell_cmd(rest):
    """validate.shell_script_tests.<stem> -> bash tests/<stem>.sh."""
    script = ROOT / "tests" / f"{rest}.sh"
    return ["bash", str(script)], "shell_script_tests"


def _wasm_cmd(rest):
    """validate.wasm_e2e.<stem> -> node web/<stem>.js (cwd web/)."""
    return ["node", f"{rest}.js"], "wasm_e2e"


def _network_cmd(rest):
    """validate.network_e2e.<deck_stem>.<seed>.

    Runs the single-scenario network gui e2e for that deck+seed. The deck stem
    is resolved against decks/ (searches subdirs since old_school decks nest).
    """
    deck_stem, _, seed = rest.rpartition(".")
    deck_path = _resolve_deck(deck_stem)
    return (["node", "test_network_gui_e2e.js", "--deck", deck_path, "--seed", seed],
            "network_e2e")


def _example_cmd(rest):
    """validate.examples.<name> -> cargo run --example <name>."""
    return ["cargo", "run", "--features", "network", "--example", rest], "examples"


KIND_RUNNERS = {
    "shell_script_tests": _shell_cmd,
    "wasm_e2e": _wasm_cmd,
    "network_e2e": _network_cmd,
    "examples": _example_cmd,
    # cargo is the fallback (kind token contains "--").
}

# wasm/network e2e run with cwd = web/.
WEB_CWD_KINDS = {"wasm_e2e", "network_e2e"}


def _resolve_deck(stem):
    """Best-effort map a deck stem back to a path under decks/."""
    decks = ROOT / "decks"
    direct = decks / f"{stem}.dck"
    if direct.exists():
        return str(direct.relative_to(ROOT))
    for p in decks.rglob(f"*{stem}*.dck"):
        return str(p.relative_to(ROOT))
    return f"decks/{stem}.dck"  # let the test report the missing file


def decode(name):
    """canonical name -> (argv, kind, cwd)."""
    if not name.startswith("validate."):
        raise ValueError(f"not a canonical name (must start with 'validate.'): {name}")
    rest = name[len("validate."):]
    kind_token, _, tail = rest.partition(".")
    if "--" in kind_token:  # cargo: validate.<pkg>--<binary>.<test>
        argv, kind = _cargo_cmd(rest)
    elif kind_token in KIND_RUNNERS:
        argv, kind = KIND_RUNNERS[kind_token](tail)
    else:
        raise ValueError(f"unknown kind '{kind_token}' in {name}")
    cwd = ROOT / "web" if kind in WEB_CWD_KINDS else ROOT
    return argv, kind, cwd


# ---------------------------------------------------------------------------
# Known canonical names for `list` and `stress-all`. Discovered dynamically
# where cheap; the network scenarios mirror web/test_network_multideck.js.
# ---------------------------------------------------------------------------

NETWORK_SCENARIOS = [
    ("monored", 13),
    ("01_rogue_rogerbrand", 3),
    ("03_robots_jesseisbak", 42),
    ("counterspells", 5),
    ("white_weenie", 7),  # known-desync mtg-273; tracked separately
]


def discover_cargo_names():
    """Enumerate unit + integration test names via
    `cargo nextest list --message-format json`.

    NOTE: this COMPILES the test binaries (can take minutes cold). Returns []
    if nextest is unavailable or the build fails — callers degrade gracefully.
    Maps each nextest suite to the canonical cargo name
    `validate.<pkg>--<binary>.<module::test>` (binary == "lib" for unit tests),
    which round-trips through decode() -> `cargo test -p <pkg> --test <bin>`.
    """
    try:
        r = subprocess.run(
            ["cargo", "nextest", "list", "--workspace", "--features", "network",
             "--message-format", "json"],
            cwd=ROOT, capture_output=True, text=True, timeout=1200)
        if r.returncode != 0 or not r.stdout.strip():
            return []
        data = json.loads(r.stdout)
    except Exception:
        return []
    names = []
    for suite in data.get("rust-suites", {}).values():
        pkg = suite.get("package-name", "")
        kind = suite.get("kind", "")
        if kind == "lib":
            binary = "lib"
        elif kind in ("test", "integration"):
            binary = suite.get("binary-name", "")
        else:
            continue  # skip bins/benches (the decoder targets lib + --test)
        if not pkg or not binary:
            continue
        for test in suite.get("testcases", {}):
            names.append(f"validate.{pkg}--{binary}.{test}")
    return names


def discover_names(include_cargo=True):
    names = []
    # unit + integration tests (compiles; skipped with --quick)
    if include_cargo:
        names += sorted(discover_cargo_names())
    # shell scripts
    for sh in sorted((ROOT / "tests").glob("*.sh")):
        names.append(f"validate.shell_script_tests.{sh.stem}")
    # wasm e2e (the explicit validate-wasm-e2e-step list lives in the Makefile;
    # we discover all web/test_*.js for `list`, but `stress-all` uses the
    # curated set below to avoid running deploy-only / network-heavy files).
    for js in sorted((ROOT / "web").glob("test_*.js")):
        names.append(f"validate.wasm_e2e.{js.stem}")
    # network scenarios
    for deck, seed in NETWORK_SCENARIOS:
        names.append(f"validate.network_e2e.{deck}.{seed}")
    return names


# Curated, light subset for stress-all (avoids the heaviest e2e by default).
WASM_E2E_VALIDATE_SET = [
    "test_fancy_tui", "test_human_input", "test_click_and_log",
    "test_font_size_layout", "test_card_size_stability",
    "test_battlefield_layout", "test_tapped_rotation", "test_graveyard_overlay",
]


def stress_all_names():
    names = [f"validate.shell_script_tests.{sh.stem}"
             for sh in sorted((ROOT / "tests").glob("*.sh"))]
    names += [f"validate.wasm_e2e.{s}" for s in WASM_E2E_VALIDATE_SET]
    return names


# ---------------------------------------------------------------------------
# Running
# ---------------------------------------------------------------------------

def run_once(argv, cwd, timeout):
    try:
        r = subprocess.run(argv, cwd=cwd, capture_output=True, text=True,
                           timeout=timeout)
        return PASS if r.returncode == 0 else FAIL
    except subprocess.TimeoutExpired:
        return TIMEOUT
    except Exception:
        return FAIL


def classify(fails, timeouts, runs):
    if fails == 0 and timeouts == 0:
        return "deterministic-pass"
    if fails == 0 and timeouts > 0:
        return "timeout-under-load"
    return "true-nondeterministic"


def stress_one(name, runs, concurrency, timeout, quiet=False):
    argv, kind, cwd = decode(name)
    if not quiet:
        cprint(CYAN, f"Stressing {name}")
        print(f"  cmd: {' '.join(argv)}  (cwd={cwd.relative_to(ROOT) if cwd != ROOT else '.'})")
        print(f"  runs={runs} concurrency={concurrency} timeout={timeout}s")
    outcomes = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as ex:
        futs = [ex.submit(run_once, argv, cwd, timeout) for _ in range(runs)]
        for i, f in enumerate(concurrent.futures.as_completed(futs)):
            o = f.result()
            outcomes.append(o)
            if not quiet:
                sym = {PASS: f"{GREEN}.{NC}", FAIL: f"{RED}F{NC}", TIMEOUT: f"{YELLOW}T{NC}"}[o]
                sys.stdout.write(sym)
                sys.stdout.flush()
    if not quiet:
        print()
    fails = outcomes.count(FAIL)
    timeouts = outcomes.count(TIMEOUT)
    pct = round(100.0 * (fails + timeouts) / runs, 2) if runs else 0.0
    cls = classify(fails, timeouts, runs)
    if not quiet:
        color = GREEN if cls == "deterministic-pass" else (YELLOW if cls == "timeout-under-load" else RED)
        cprint(color, f"  {name}: {runs} runs, {fails} fail, {timeouts} timeout "
                      f"-> {pct}% flaky [{cls}]")
    return {"kind": kind, "runs": runs, "fails": fails, "timeouts": timeouts,
            "pct": pct, "classification": cls, "concurrency": concurrency}


# ---------------------------------------------------------------------------
# DB
# ---------------------------------------------------------------------------

def db_path():
    return ROOT / DB_PATH_REL


def ensure_db():
    p = db_path()
    if not p.exists():
        p.parent.mkdir(parents=True, exist_ok=True)
        with p.open("w", newline="") as f:
            csv.writer(f).writerow(DB_HEADER)


def record(name, res, issue="", notes=""):
    ensure_db()
    row = [
        datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        git_sha(), git_depth(), cpu_name(), name, res["kind"], res["runs"],
        res["fails"], res["timeouts"], res["pct"], res["classification"], issue,
        res["concurrency"], notes,
    ]
    with db_path().open("a", newline="") as f:
        csv.writer(f).writerow(row)
    cprint(CYAN, f"  recorded -> {DB_PATH_REL}")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def default_concurrency():
    return max(1, min(4, (os.cpu_count() or 2) // 2))


def main():
    ap = argparse.ArgumentParser(description="Test-flakiness stress harness (mtg-j010x)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    one = sub.add_parser("one", help="stress a single canonical test")
    one.add_argument("name")
    one.add_argument("--runs", type=int, default=10)
    one.add_argument("--concurrency", type=int, default=default_concurrency())
    one.add_argument("--timeout", type=int, default=300)
    one.add_argument("--record", action="store_true")
    one.add_argument("--classify", dest="cls_override", default=None,
                     help="override auto-classification (e.g. known-desync)")
    one.add_argument("--issue", default="", help="linked beads issue")
    one.add_argument("--notes", default="")

    sa = sub.add_parser("stress-all", help="bounded sweep over the known suite")
    sa.add_argument("--runs", type=int, default=3)
    sa.add_argument("--concurrency", type=int, default=default_concurrency())
    sa.add_argument("--timeout", type=int, default=300)
    sa.add_argument("--record", action="store_true")

    ls = sub.add_parser("list", help="list known canonical names")
    ls.add_argument("--quick", action="store_true",
                    help="skip cargo unit/integration tests (no compile)")

    args = ap.parse_args()

    if args.cmd == "list":
        if not args.quick:
            cprint(CYAN, "# enumerating cargo unit/integration tests "
                         "(compiles test binaries; use --quick to skip)...")
        for n in discover_names(include_cargo=not args.quick):
            print(n)
        return

    if args.cmd == "one":
        res = stress_one(args.name, args.runs, args.concurrency, args.timeout)
        if args.cls_override:
            res["classification"] = args.cls_override
        if args.record:
            record(args.name, res, issue=args.issue, notes=args.notes)
        sys.exit(1 if res["classification"] == "true-nondeterministic" else 0)

    if args.cmd == "stress-all":
        names = stress_all_names()
        cprint(CYAN, f"stress-all: {len(names)} tests x {args.runs} runs "
                     f"(concurrency {args.concurrency})")
        worst = "deterministic-pass"
        for n in names:
            try:
                res = stress_one(n, args.runs, args.concurrency, args.timeout, quiet=False)
            except Exception as e:
                cprint(RED, f"  {n}: decode/run error: {e}")
                continue
            if args.record:
                record(n, res)
            if res["classification"] == "true-nondeterministic":
                worst = "true-nondeterministic"
        sys.exit(1 if worst == "true-nondeterministic" else 0)


if __name__ == "__main__":
    main()
