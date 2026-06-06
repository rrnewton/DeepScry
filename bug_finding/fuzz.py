#!/usr/bin/env python3
"""
fuzz.py — the ONE bug-finding / fuzz / stress entrypoint for mtg-forge-rs.

This is a single CLI with one subcommand per MODE. It REUSES the shared layer
(`network_test_lib.py` for the loopback/local game runners + gamelog oracles)
and DELEGATES the specialised harnesses (native-vs-WASM sweep, snapshot stress,
snapshot-determinism, flakiness) to their existing modules rather than
re-implementing them — one implementation per distinct comparison semantic
(DRY). See docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md for the policy.

This is a BUG-FINDING tool, NOT a regression test. `make validate` runs the
deterministic fixed-seed legs (tests/*_e2e.sh, the bash determinism/equiv
sweep, the native-vs-WASM validate leg). This driver sweeps MANY random seeds /
decks / stop-points to surface NEW desyncs — the mtg-813 "1-hour expedition"
prize (successor to mtg-q97bw).

MODES (subcommands):
  network               network-only fuzz: run loopback games, flag crashes/errors
  equivalence           local==network gamelog identity (the desync hunt)
  determinism           native same-seed determinism (one deck twice, local)
  native-wasm           native==WASM strict engine-equivalence sweep
  snapshot              snapshot/resume stress over deck(s) x matchup(s)
  snapshot-determinism  snapshot-twice-from-same-state determinism
  flakiness             repeated-run flakiness of an EXISTING canonical test
  expedition            wall-clock budget driver: old-school corpus x config
                        matrix, all-debug-on, aggregate findings + reproducers

Examples:
  python3 bug_finding/fuzz.py determinism --seeds 20 --decks 'decks/old_school2/*.dck'
  python3 bug_finding/fuzz.py equivalence --configs 50 --client native
  python3 bug_finding/fuzz.py equivalence --configs 20 --client wasm
  python3 bug_finding/fuzz.py network --infinite
  python3 bug_finding/fuzz.py native-wasm --seeds 50
  python3 bug_finding/fuzz.py snapshot --decks royal_assassin,monored --matchups heuristic:heuristic
  python3 bug_finding/fuzz.py snapshot-determinism decks/monored.dck --choice 5 10
  python3 bug_finding/fuzz.py flakiness one validate.shell_script_tests.commander_e2e --runs 20
  python3 bug_finding/fuzz.py expedition --duration 3600 --modes determinism,equivalence
"""

import argparse
import glob
import os
import random
import shutil
import signal
import sys
import time
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Callable, Dict, List, Optional, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from network_test_lib import (  # noqa: E402
    MTG_BIN, WORKSPACE_ROOT, CONTROLLERS,
    TestConfig, TestResult,
    run_network_test, run_equivalence_test, run_determinism_test,
)

# The canonical 1994 old-school bug-finding corpus (mtg-813 prize target).
DEFAULT_DECK_GLOBS = "decks/old_school/*.dck,decks/old_school2/*.dck"
DEFAULT_SEED_BASE = 1

# Graceful-shutdown state (Ctrl-C prints a summary instead of a stack trace).
_shutdown = False


def _install_signal_handlers() -> None:
    def handler(signum, frame):
        global _shutdown
        if _shutdown:
            print("\nForce quit.")
            sys.exit(1)
        print("\n\nShutdown requested — finishing in-flight runs, then summary...")
        _shutdown = True
    signal.signal(signal.SIGINT, handler)
    signal.signal(signal.SIGTERM, handler)


# ═══════════════════════════════════════════════════════════════════════════
# Config-matrix construction (shared by the inline game modes)
# ═══════════════════════════════════════════════════════════════════════════

def _expand_decks(deck_globs: str) -> List[str]:
    """Expand comma-separated shell globs (relative to the repo root) to a
    sorted, de-duplicated list of absolute .dck paths."""
    paths: List[str] = []
    seen = set()
    for g in deck_globs.split(","):
        g = g.strip()
        if not g:
            continue
        if not os.path.isabs(g):
            g = os.path.join(WORKSPACE_ROOT, g)
        for p in sorted(glob.glob(g)):
            if p not in seen:
                seen.add(p)
                paths.append(p)
    return paths


def _deck_pairs(decks: List[str], pair_mode: str,
                max_pairs: int = 0) -> List[Tuple[str, str]]:
    """Form deck pairs from a corpus.

    chain (default): consecutive pairs (d0,d1),(d1,d2)... ~N pairs.
    all:            every unordered pair i<j (O(n^2)) — heavy overnight runs.
    self:           each deck mirror-matched against itself.
    """
    pairs: List[Tuple[str, str]] = []
    if not decks:
        return pairs
    if pair_mode == "self":
        pairs = [(d, d) for d in decks]
    elif pair_mode == "all":
        for i in range(len(decks)):
            for j in range(i + 1, len(decks)):
                pairs.append((decks[i], decks[j]))
        if not pairs:  # single-deck corpus: fall back to a mirror
            pairs = [(decks[0], decks[0])]
    else:  # chain
        if len(decks) == 1:
            pairs = [(decks[0], decks[0])]
        else:
            pairs = [(decks[i], decks[i + 1]) for i in range(len(decks) - 1)]
    if max_pairs > 0:
        pairs = pairs[:max_pairs]
    return pairs


def _client_pairs(client: str) -> List[Tuple[str, str]]:
    if client == "wasm":
        return [("wasm", "wasm")]
    if client == "mixed":
        return [("native", "wasm"), ("wasm", "native")]
    return [("native", "native")]


def build_configs(deck_globs: str, seed_base: int, num_seeds: int,
                  controllers: List[str], client: str,
                  pair_mode: str, max_pairs: int) -> List[TestConfig]:
    """Cartesian product: deck-pairs x seeds x controllers x client-modes."""
    decks = _expand_decks(deck_globs)
    if not decks:
        raise SystemExit(f"ERROR: no decks matched globs: {deck_globs}")
    pairs = _deck_pairs(decks, pair_mode, max_pairs)
    seeds = [seed_base + i for i in range(num_seeds)]
    cl_pairs = _client_pairs(client)
    configs: List[TestConfig] = []
    for (d1, d2) in pairs:
        for c in controllers:
            for seed in seeds:
                for (cl1, cl2) in cl_pairs:
                    configs.append(TestConfig(
                        seed=seed, controller_p1=c, controller_p2=c,
                        deck1=d1, deck2=d2, client_p1=cl1, client_p2=cl2,
                    ))
    return configs


# ═══════════════════════════════════════════════════════════════════════════
# Generic parallel runner + summary (folded in from network_fuzz_test.py)
# ═══════════════════════════════════════════════════════════════════════════

def _repro_for(mode: str, cfg: TestConfig) -> str:
    """A deterministic single-seed reproducer command for a finding."""
    d1 = os.path.relpath(cfg.deck1, WORKSPACE_ROOT)
    d2 = os.path.relpath(cfg.deck2, WORKSPACE_ROOT)
    if mode == "determinism":
        return (f"{MTG_BIN} tui {d1} {d2} --p1 {cfg.controller_p1} "
                f"--p2 {cfg.controller_p2} --seed {cfg.seed} --tag-gamelogs "
                f"# run twice, diff [GAMELOG] lines")
    # The fixed-deck e2e (tests/network_vs_local_equivalence_e2e.sh) only covers
    # the avatar pair, so a faithful reproducer for an arbitrary deck pair is the
    # single-seed fuzz.py invocation that carries the exact decks/seed/controller.
    client = f" --client {cfg.client_p1}" if cfg.client_p1 != "native" else ""
    return (f"python3 bug_finding/fuzz.py {mode} --decks '{d1},{d2}' "
            f"--seed-base {cfg.seed} --seeds 1 --controllers {cfg.controller_p1}{client}")


def run_batch(mode: str, configs: List[TestConfig],
              runner: Callable[[TestConfig], TestResult],
              parallel: int, debug_dir: Optional[str] = None) -> List[TestResult]:
    """Run configs concurrently, stream PASS/FAIL, return all results.

    Passing runs have their temp output dirs cleaned; failing runs are kept (or
    copied into debug_dir when provided) so a finding can be triaged.
    """
    results: List[TestResult] = []
    total = len(configs)
    with ThreadPoolExecutor(max_workers=parallel) as ex:
        futures = {ex.submit(runner, c): c for c in configs}
        for i, fut in enumerate(as_completed(futures)):
            if _shutdown:
                break
            res = fut.result()
            results.append(res)
            tag = "PASS" if res.passed else f"FAIL ({res.error_signature})"
            print(f"  [{i + 1}/{total}] {res.config}: {tag} ({res.duration:.1f}s)")
            if res.passed:
                if res.output_dir and os.path.isdir(res.output_dir):
                    shutil.rmtree(res.output_dir, ignore_errors=True)
            else:
                print(f"      repro: {_repro_for(mode, res.config)}")
                if res.gamelog_diff_sample:
                    for ln in res.gamelog_diff_sample.split("\n")[:6]:
                        print(f"      {ln}")
                if debug_dir and res.output_dir and os.path.isdir(res.output_dir):
                    dest = os.path.join(debug_dir, os.path.basename(res.output_dir))
                    try:
                        shutil.copytree(res.output_dir, dest, dirs_exist_ok=True)
                    except Exception:
                        pass
    return results


def print_summary(mode: str, results: List[TestResult]) -> int:
    """Print a summary; return the number of failures."""
    if not results:
        print("\nNo runs completed.")
        return 0
    passed = sum(1 for r in results if r.passed)
    failed = len(results) - passed
    print()
    print(f"=== Summary ({mode}) ===")
    print(f"Total:  {len(results)}")
    print(f"Passed: {passed} ({100 * passed / len(results):.1f}%)")
    print(f"Failed: {failed} ({100 * failed / len(results):.1f}%)")

    if failed:
        buckets: Dict[str, List[TestResult]] = defaultdict(list)
        for r in results:
            if not r.passed:
                buckets[r.error_signature or "unknown"].append(r)
        print("\n=== Findings (by signature) ===")
        for sig, rs in sorted(buckets.items(), key=lambda kv: -len(kv[1])):
            ex = rs[0]
            print(f"\n{sig}: {len(rs)} occurrence(s)")
            print(f"  example: {ex.config}")
            print(f"  repro:   {_repro_for(mode, ex.config)}")
            if ex.output_dir and os.path.isdir(ex.output_dir):
                print(f"  logs:    {ex.output_dir}")
    return failed


# ═══════════════════════════════════════════════════════════════════════════
# Inline game modes: network / equivalence / determinism
# ═══════════════════════════════════════════════════════════════════════════

_RUNNERS: Dict[str, Callable[[TestConfig, int], TestResult]] = {
    "network": run_network_test,
    "equivalence": run_equivalence_test,
    "determinism": run_determinism_test,
}


def _common_game_args(p: argparse.ArgumentParser) -> None:
    p.add_argument("--decks", default=DEFAULT_DECK_GLOBS,
                   help=f"Comma-separated deck glob(s). Default: {DEFAULT_DECK_GLOBS}")
    p.add_argument("--seeds", type=int, default=5, help="Seeds per deck-pair x controller.")
    p.add_argument("--seed-base", type=int, default=DEFAULT_SEED_BASE,
                   help="First seed value (inclusive).")
    p.add_argument("--controllers", default="heuristic random",
                   help='Space-separated controllers (each runs BOTH players). '
                        'Valid: heuristic random zero. Default: "heuristic random".')
    p.add_argument("--pair-mode", choices=["chain", "all", "self"], default="chain",
                   help="How to form deck pairs from the corpus (default: chain).")
    p.add_argument("--max-pairs", type=int, default=0, help="Cap deck pairs (0=no cap).")
    p.add_argument("--parallel", type=int, default=3, help="Concurrent runs (keep low).")
    p.add_argument("--timeout", type=int, default=180, help="Per-game timeout (s).")
    p.add_argument("--debug-dir", default=None,
                   help="Copy failing-run logs here (gitignored debug/ recommended).")


def _common_game_args_with_clients(p: argparse.ArgumentParser) -> None:
    _common_game_args(p)
    p.add_argument("--client", choices=["native", "wasm", "mixed"], default="native",
                   help="Network client mode (network/equivalence only).")
    p.add_argument("--configs", type=int, default=0,
                   help="Cap total configs after expansion (0=all). Shuffled when capped.")
    p.add_argument("--infinite", action="store_true", help="Loop batches until Ctrl-C.")
    p.add_argument("--duration", type=int, default=0,
                   help="Run batches for this many seconds then stop (0=one batch).")


def cmd_inline_game(mode: str, args: argparse.Namespace) -> int:
    if not os.path.exists(MTG_BIN):
        print(f"ERROR: binary not found: {MTG_BIN}\n"
              "Build it: cargo build --release --features network", file=sys.stderr)
        return 2
    controllers = args.controllers.split()
    for c in controllers:
        if c not in CONTROLLERS:
            print(f"ERROR: invalid controller {c!r} (valid: {CONTROLLERS})", file=sys.stderr)
            return 2
    client = getattr(args, "client", "native")
    if mode == "determinism" and client != "native":
        print("NOTE: determinism mode is local-only; ignoring --client.", file=sys.stderr)
        client = "native"
    if args.debug_dir:
        os.makedirs(args.debug_dir, exist_ok=True)

    runner = lambda cfg: _RUNNERS[mode](cfg, args.timeout)  # noqa: E731

    print(f"=== fuzz.py {mode} ===")
    print(f"  decks={args.decks}  seeds={args.seeds}@{args.seed_base}  "
          f"controllers={controllers}  pair-mode={args.pair_mode}  client={client}")

    all_results: List[TestResult] = []
    start = time.time()
    infinite = getattr(args, "infinite", False)
    duration = getattr(args, "duration", 0)
    cap = getattr(args, "configs", 0)
    batch = 0
    while not _shutdown:
        batch += 1
        configs = build_configs(args.decks, args.seed_base, args.seeds,
                                controllers, client, args.pair_mode, args.max_pairs)
        if infinite or duration > 0:
            # Randomise seeds each batch so a long run keeps exploring.
            for cfg in configs:
                cfg.seed = random.randint(1, 1_000_000)
            random.shuffle(configs)
        if cap > 0 and len(configs) > cap:
            random.shuffle(configs)
            configs = configs[:cap]
        print(f"\n--- batch {batch}: {len(configs)} configs "
              f"(elapsed {time.time() - start:.0f}s) ---")
        all_results.extend(run_batch(mode, configs, runner, args.parallel, args.debug_dir))
        if not infinite and duration == 0:
            break
        if duration > 0 and (time.time() - start) >= duration:
            print(f"\nDuration limit reached ({duration}s).")
            break

    failed = print_summary(mode, all_results)
    return 1 if failed else 0


# ═══════════════════════════════════════════════════════════════════════════
# Delegated modes (keep their own argparse; one entrypoint forwards to them)
# ═══════════════════════════════════════════════════════════════════════════

def _forward(module_name: str, argv: List[str]) -> int:
    """Run an existing harness module's main() with a swapped sys.argv.

    Normalises both `return int` and `sys.exit(int)` styles to an exit code so
    the unified CLI can forward to harnesses that pre-date it without rewriting
    them (DRY: one implementation per comparison semantic).
    """
    import importlib
    mod = importlib.import_module(module_name)
    saved = sys.argv
    sys.argv = [f"fuzz.py {module_name}"] + argv
    try:
        rc = mod.main()
        return int(rc) if isinstance(rc, int) else 0
    except SystemExit as e:
        return int(e.code) if isinstance(e.code, int) else (0 if e.code is None else 1)
    finally:
        sys.argv = saved


def cmd_native_wasm(extra: List[str]) -> int:
    import native_wasm_equiv_sweep
    return int(native_wasm_equiv_sweep.main(extra))


def cmd_snapshot(args: argparse.Namespace, extra: List[str]) -> int:
    """Snapshot/resume stress over deck(s) x matchup(s).

    Subsumes tests/disabled/run_stress_tests.sh (a GNU-parallel loop over the
    same single-deck harness) by looping the deck x matchup grid inside the one
    CLI and calling snapshot_stress_test_single.run_test_for_deck directly.
    """
    import snapshot_stress_test_single as sss
    mtg_bin = sss.find_mtg_binary()
    deck_args = [d.strip() for d in args.decks.split(",") if d.strip()]
    deck_paths: List[str] = []
    for d in deck_args:
        # Accept either a bare deck name or a path (with/without .dck).
        cand = d if os.path.isabs(d) else os.path.join(WORKSPACE_ROOT, d)
        if not cand.endswith(".dck"):
            cand += ".dck"
        if not os.path.exists(cand):
            cand2 = os.path.join(WORKSPACE_ROOT, "decks", os.path.basename(d))
            if not cand2.endswith(".dck"):
                cand2 += ".dck"
            cand = cand2
        deck_paths.append(cand)

    matchups = []
    for m in args.matchups.split(","):
        m = m.strip()
        if not m:
            continue
        parts = m.split(":")
        p1, p2 = parts[0], parts[1] if len(parts) > 1 else parts[0]
        switch_fixed = len(parts) > 2 and parts[2] == "--switch-fixed"
        matchups.append((p1, p2, switch_fixed))

    failures = 0
    total = 0
    for deck in deck_paths:
        if not os.path.exists(deck):
            print(f"ERROR: deck not found: {deck}", file=sys.stderr)
            failures += 1
            continue
        for (p1, p2, switch_fixed) in matchups:
            total += 1
            label = f"{os.path.basename(deck)} ({p1} vs {p2}{' switch-fixed' if switch_fixed else ''})"
            print(f"--- snapshot stress: {label} ---")
            ok = sss.run_test_for_deck(
                mtg_bin, deck, p1, p2, args.seed,
                num_replays=args.replays, verbose=args.verbose,
                switch_fixed=switch_fixed, json_format=args.json,
            )
            print(("  ✓ " if ok else "  ✗ ") + label)
            if not ok:
                failures += 1
            if _shutdown:
                break
    print(f"\n=== snapshot summary: {total - failures}/{total} passed ===")
    return 1 if failures else 0


# ═══════════════════════════════════════════════════════════════════════════
# Expedition mode: the mtg-813 1-hour prize driver
# ═══════════════════════════════════════════════════════════════════════════

def cmd_expedition(args: argparse.Namespace) -> int:
    """Wall-clock-budgeted bug hunt over the old-school corpus x config matrix.

    Rotates the requested check modes across random seeds + deck pairs until the
    duration budget is exhausted, aggregating findings with per-finding
    reproducers. All debug checks are ON (network runs use --network-debug; the
    maximally-strict state hash incl. action_count is engine-default
    post-1070b585). This is the prize harness: an hour with ZERO desyncs.
    """
    if not os.path.exists(MTG_BIN):
        print(f"ERROR: binary not found: {MTG_BIN}", file=sys.stderr)
        return 2
    modes = [m.strip() for m in args.modes.split(",") if m.strip()]
    for m in modes:
        if m not in _RUNNERS:
            print(f"ERROR: expedition mode {m!r} not one of {list(_RUNNERS)}",
                  file=sys.stderr)
            return 2
    controllers = args.controllers.split()
    decks = _expand_decks(args.decks)
    if not decks:
        print(f"ERROR: no decks matched {args.decks}", file=sys.stderr)
        return 2
    pairs = _deck_pairs(decks, args.pair_mode, args.max_pairs)
    debug_dir = args.debug_dir or os.path.join(
        WORKSPACE_ROOT, "debug", "expedition")
    os.makedirs(debug_dir, exist_ok=True)

    print("=== fuzz.py expedition (mtg-813 prize driver) ===")
    print(f"  budget={args.duration}s  modes={modes}  decks={len(decks)} "
          f"pairs={len(pairs)}  controllers={controllers}  parallel={args.parallel}")
    print(f"  findings -> {debug_dir}")

    start = time.time()
    all_results: List[TestResult] = []
    round_n = 0
    while not _shutdown and (time.time() - start) < args.duration:
        round_n += 1
        # One round = one randomized config per (mode x pair x controller),
        # capped, run in parallel. Seeds are random so each round explores new
        # ground.
        configs: List[Tuple[str, TestConfig]] = []
        for mode in modes:
            client = args.client if mode != "determinism" else "native"
            cl_pairs = _client_pairs(client)
            for (d1, d2) in pairs:
                for c in controllers:
                    for (cl1, cl2) in cl_pairs:
                        configs.append((mode, TestConfig(
                            seed=random.randint(1, 1_000_000),
                            controller_p1=c, controller_p2=c,
                            deck1=d1, deck2=d2, client_p1=cl1, client_p2=cl2,
                        )))
        random.shuffle(configs)
        remaining = args.duration - (time.time() - start)
        print(f"\n--- round {round_n}: {len(configs)} configs "
              f"({remaining:.0f}s budget left) ---")

        def _run(item):
            mode, cfg = item
            return mode, _RUNNERS[mode](cfg, args.timeout)

        with ThreadPoolExecutor(max_workers=args.parallel) as ex:
            futures = {ex.submit(_run, it): it for it in configs}
            for fut in as_completed(futures):
                if _shutdown or (time.time() - start) >= args.duration:
                    break
                mode, res = fut.result()
                all_results.append(res)
                if res.passed:
                    if res.output_dir and os.path.isdir(res.output_dir):
                        shutil.rmtree(res.output_dir, ignore_errors=True)
                else:
                    print(f"  FINDING [{mode}] {res.config}: {res.error_signature}")
                    print(f"    repro: {_repro_for(mode, res.config)}")
                    if res.output_dir and os.path.isdir(res.output_dir):
                        dest = os.path.join(debug_dir, os.path.basename(res.output_dir))
                        try:
                            shutil.copytree(res.output_dir, dest, dirs_exist_ok=True)
                        except Exception:
                            pass

    elapsed = time.time() - start
    print(f"\n=== expedition complete: {elapsed:.0f}s, {round_n} round(s), "
          f"{len(all_results)} games ===")
    failed = print_summary("expedition", all_results)
    if failed == 0:
        print("\n✓ ZERO desyncs found — prize condition met for this run.")
    return 1 if failed else 0


# ═══════════════════════════════════════════════════════════════════════════
# Argument parsing / dispatch
# ═══════════════════════════════════════════════════════════════════════════

def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="fuzz.py",
        description="Unified bug-finding / fuzz / stress driver (NOT a regression test).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = p.add_subparsers(dest="mode", required=True)

    for mode in ("network", "equivalence"):
        sp = sub.add_parser(mode, help=f"{mode} fuzz over the loopback harness")
        _common_game_args_with_clients(sp)

    det = sub.add_parser("determinism", help="native same-seed determinism (local)")
    _common_game_args_with_clients(det)

    sub.add_parser("native-wasm",
                   help="native==WASM strict sweep (forwards to native_wasm_equiv_sweep)",
                   add_help=False)

    snap = sub.add_parser("snapshot", help="snapshot/resume stress over decks x matchups")
    snap.add_argument("--decks", default="royal_assassin,white_aggro_4ed,monored",
                      help="Comma-separated deck names or paths.")
    snap.add_argument("--matchups", default="heuristic:heuristic,random:heuristic",
                      help="Comma-separated p1:p2[:--switch-fixed] matchups.")
    snap.add_argument("--seed", type=int, default=42)
    snap.add_argument("--replays", type=int, default=3)
    snap.add_argument("--json", action="store_true", help="JSON snapshots (default binary).")
    snap.add_argument("--verbose", "-v", action="store_true")

    sub.add_parser("snapshot-determinism",
                   help="snapshot-twice determinism (forwards to test_snapshot_determinism)",
                   add_help=False)
    sub.add_parser("flakiness",
                   help="repeated-run flakiness of an existing test (forwards to flakiness_stress)",
                   add_help=False)

    exp = sub.add_parser("expedition", help="1-hour bug-finding expedition (mtg-813 prize)")
    exp.add_argument("--duration", type=int, default=3600, help="Wall-clock budget (s).")
    exp.add_argument("--modes", default="determinism,equivalence",
                     help="Comma-separated check modes: determinism,equivalence,network.")
    exp.add_argument("--decks", default=DEFAULT_DECK_GLOBS)
    exp.add_argument("--controllers", default="heuristic random")
    exp.add_argument("--client", choices=["native", "wasm", "mixed"], default="native")
    exp.add_argument("--pair-mode", choices=["chain", "all", "self"], default="chain")
    exp.add_argument("--max-pairs", type=int, default=0)
    exp.add_argument("--parallel", type=int, default=3)
    exp.add_argument("--timeout", type=int, default=180)
    exp.add_argument("--debug-dir", default=None)
    return p


# Modes that own their argparse — we forward the raw remainder to them.
_FORWARD_MODES = {
    "native-wasm": lambda extra: cmd_native_wasm(extra),
    "snapshot-determinism": lambda extra: _forward("test_snapshot_determinism", extra),
    "flakiness": lambda extra: _forward("flakiness_stress", extra),
}


def main(argv: Optional[List[str]] = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    _install_signal_handlers()

    mode = argv[0] if argv else None
    # Forwarded modes: pass everything after the subcommand verbatim so the
    # delegated harness's own --help/flags work unchanged.
    if mode in _FORWARD_MODES:
        return _FORWARD_MODES[mode](argv[1:])

    parser = build_parser()
    args = parser.parse_args(argv)

    if args.mode in _RUNNERS:
        return cmd_inline_game(args.mode, args)
    if args.mode == "snapshot":
        return cmd_snapshot(args, [])
    if args.mode == "expedition":
        return cmd_expedition(args)
    parser.error(f"unhandled mode {args.mode}")
    return 2


if __name__ == "__main__":
    sys.exit(main())
