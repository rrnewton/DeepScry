#!/usr/bin/env python3
"""validate.py — the `make validate` entry point (mtg-717 + follow-on).

This is the SINGLE SOURCE OF TRUTH for what `make validate` runs and how. It
replaced the Makefile's `validate-*-step` / `validate-parallel-steps` /
`validate-impl` apparatus and `scripts/validate_step.sh` (the orchestrator), and
then ABSORBED the former `scripts/validate.sh` outer harness too — so there is
now ONE file, not bash-wrapping-python. The Makefile keeps only thin
build-primitive targets (build, wasm-dev, clippy, …) that this runner invokes;
CI shards call THIS with `--group <X>` so CI can never drift from local.

Two layers in one file:
  * the OUTER HARNESS (run_with_harness) — commit-hash cache + docs-only smart
    hit, `.validate.lock`, dirty-tree→temporary WIP-commit, clean-environment
    gate, CPU-utilization report, atomic validate_logs/validate_<sha>.log
    artifact + latest symlink. Runs ONLY for a full local `make validate`.
  * the ORCHESTRATOR (run_orchestrator) — everything below. A subset run
    (--group/--only/--job), --list, or --use-prebuilt skips the harness and runs
    this directly, so CI shards stay hermetic (no cache/lock/WIP).

The orchestrator owns:
  * the STEP REGISTRY: (jobGroup, jobId) -> command, deps, resources, profile;
  * a dependency- AND resource-aware PARALLEL SCHEDULER (build-once falls out of
    deps; a capacity-limited "browser" resource serializes chromium-heavy steps
    to avoid over-subscription / timing flakes while CPU work runs concurrently);
  * LOG HYGIENE: terse tagged `[jobGroup.jobId]` lines; per-step detail streamed
    to validate_logs/steps_<sha>/<group>.<job>.log (ALWAYS persisted); a one-line
    per-step SUMMARY by default (e.g. nextest "N passed"); detail dumped INTO the
    log on FAILURE (self-contained);
  * VERBOSITY levels (-q / default / -v / -vv);
  * SUBSET filtering (--only g.j,…  --group g,…  --job j,…);
  * an end-of-run STATS block: total wall-clock, serial-sum, PARALLEL SPEEDUP,
    per-group breakdown, slowest steps (critical path).

Usage:
  validate.py [--jobs N] [--group G[,G2]] [--only G.J[,…]] [--job J[,…]]
              [-q|-v|-vv] [--list] [--no-network] [--no-wasm-e2e]
              [--browser-capacity N] [--use-prebuilt]
              [--force] [--sequential] [--no-wip-commit] [--no-harness]

Exit status: 0 if all selected steps pass, 1 otherwise.
"""

import argparse
import os
import shutil
import re
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

# Two-level cgroup teardown (per-step child cgroups under the outer scope). Kept
# in a sibling module to keep this file focused; degrades to None if unavailable
# so the killpg reaper remains the sole backstop on hosts without it.
try:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import validate_cgroup
except Exception:  # pragma: no cover - defensive: never let an import break validate
    validate_cgroup = None

# Per-step wall-clock cap. A step that exceeds it is SIGKILLed and marked FAIL —
# so a hung step fails its shard FAST + visibly instead of dragging CI for hours
# (the 40-min default let a single stuck step eat a whole run). Default is tight
# (TEST steps should finish in minutes even on a 2-vCPU runner); the COLD-build
# steps (build.mtg-release, wasm.bundle, unit.nextest) get an explicit larger
# `timeout=` in the registry so a slow cold compile on 2 vCPU isn't falsely
# killed. Override globally via VALIDATE_STEP_TIMEOUT.
DEFAULT_STEP_TIMEOUT = int(os.environ.get("VALIDATE_STEP_TIMEOUT", "600"))
BUILD_STEP_TIMEOUT = int(os.environ.get("VALIDATE_BUILD_TIMEOUT", "2400"))

PROJECT_DIR = Path(__file__).resolve().parent.parent
NODE = os.environ.get("NODE") or "node"
NPM = os.environ.get("NPM") or "npm"


# ===========================================================================
# MEMORY-CAP BASELINES  (THE single source of truth — mtg-887)
# ===========================================================================
# `make validate` runs under cgroup memory caps BY DEFAULT so a runaway test
# (e.g. the 2026-06-09 Return-the-Favor infinite self-copy that ballooned one
# `mtg` to ~40 GB and wedged the box) is cgroup-OOM-killed at its cap instead of
# taking the host down. Caps are set at FACTOR x a characterized baseline. THESE
# CONSTANTS are the one place those baselines live; the actionable-OOM message
# points agents HERE (file + symbol) to confirm-and-bump after proving genuine
# growth (NOT an unbounded leak).
#
# HOW THE BASELINES WERE MEASURED:
#   * Total (whole-run scope) peak RSS read from the outer-scope cgroup
#     `memory.peak` after a full `make validate -j16` (validate.py prints it).
#     Observed ~22 GiB on the 16-core dev box (2026-06-09).
#   * Per-step peaks read from each step's child-cgroup `memory.peak` at step
#     teardown (printed in the per-step detail + the VALIDATE STATS block).
# CAVEAT (mtg-887 item 3) — RESOLVED 2026-06-09: a baseline measured while the
# commander runaway (Return-the-Favor self-copy loop) was live would be GARBAGE.
# That loop is now FIXED on integration (slot01 @2c9d1808 — the chandra_tokens/
# seed-42 game that spawned 419,145 copies completes with ZERO copy events), so
# determ.commander has been re-measured CLEAN (~31 MB, not the ~40 GB runaway)
# and is now in PER_STEP_RSS_BASELINE with its real cap.
MEM_CAP_FACTOR = float(os.environ.get("VALIDATE_MEM_CAP_FACTOR", "1.25"))  # 1.25x; relax to 1.5 only if too tight
# Characterized typical whole-run peak RSS (bytes). Override via env for a
# differently-sized box. 24 GiB ~= the observed ~22 GiB rounded up a touch.
VALIDATE_TOTAL_RSS_BASELINE_BYTES = int(
    os.environ.get("VALIDATE_TOTAL_RSS_BASELINE_BYTES", str(24 * 1024**3)))
# Never cap below this — a tiny cap would false-OOM a legitimate run on a small box.
MEM_CAP_FLOOR_BYTES = int(os.environ.get("VALIDATE_MEM_CAP_FLOOR_BYTES", str(8 * 1024**3)))
# Per-step characterized typical peak RSS (bytes), keyed by step tag. Each step's
# INNER cgroup MemoryMax = FACTOR x this. A step ABSENT here gets NO inner cap
# (outer cap still applies) — used for steps not yet characterized OR deliberately
# excluded (determ.commander, until slot01's runaway fix lands). Conservative
# values from the 2026-06-09 -j16 run; bump per the actionable-OOM message.
PER_STEP_RSS_BASELINE = {
    # MEASURED typical peaks from the 2026-06-09 -j16 run @d6dc7897 (the FIRST
    # run with real inner cgroups + the commander loop fixed) — each step's
    # cgroup memory.peak, printed into its detail. Rounded UP for headroom (the
    # heavy compile steps vary with build-cache state, so they keep a bit more).
    # the heavy compilers / test binaries  (measured: build 4.1G, nextest 6.8G,
    #   wasm.bundle 3.1G, clippy 2.7G, clippy-wasm 1.2G, examples 4.5G)
    "build.mtg-release": 5 * 1024**3,   # peak 4.1G
    "unit.nextest":      8 * 1024**3,   # peak 6.8G (the hungriest; extra headroom)
    "wasm.bundle":       4 * 1024**3,   # peak 3.1G
    "lint.clippy":       4 * 1024**3,   # peak 2.7G
    "lint.clippy-wasm":  2 * 1024**3,   # peak 1.2G
    "examples.run":      5 * 1024**3,   # peak 4.5G
    # commander: REAL baseline now the Return-the-Favor loop is fixed (slot01
    #   @2c9d1808) — the clean game peaks at ~31 MB, NOT the ~40 GB runaway.
    "determ.commander":  512 * 1024**2,  # peak 31.2M (generous floor for a tiny step)
    # browser/network steps (chromium + server + AI peer)  (measured: browser
    #   0.7G, multideck 3.2G, gui 0.9G, equiv sweeps ~0.6-1.2G)
    "wasm.browser":      2 * 1024**3,   # peak 694M
    "network.multideck": 4 * 1024**3,   # peak 3.2G
    "network.gui":       2 * 1024**3,   # peak 901M
}


def _total_ram_bytes():
    """Total physical RAM (bytes) from /proc/meminfo MemTotal. None if unreadable."""
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def step_mem_cap_bytes(tag):
    """Inner-cgroup MemoryMax (bytes) for a step, or None if the step is not
    characterized / deliberately excluded (then only the outer cap protects it).
    FACTOR x the per-step baseline from PER_STEP_RSS_BASELINE (the single source
    of truth, above)."""
    base = PER_STEP_RSS_BASELINE.get(tag)
    return int(base * MEM_CAP_FACTOR) if base else None


# ---------------------------------------------------------------------------
# Memory FOOTPRINT model (single source of truth = the constants above).
# "How much RAM does `validate -jN` need, worst case?" and its inverse
# "what is the largest -jN whose footprint fits in budget M?". Both derive
# straight from PER_STEP_RSS_BASELINE + the outer cap — NO flat per-job estimate.
# ---------------------------------------------------------------------------
def outer_cap_bytes():
    """The outer-scope MemoryMax baseline = whole-run peak baseline x FACTOR.
    This is the absolute ceiling on a run's footprint regardless of -j (the run
    can never use more than the outer cgroup permits). NOT clamped to RAM here —
    that clamp is a separate concern applied when the scope is actually created."""
    return int(VALIDATE_TOTAL_RSS_BASELINE_BYTES * MEM_CAP_FACTOR)


def _sorted_step_caps():
    """Per-step inner caps (bytes), descending. Steps absent from
    PER_STEP_RSS_BASELINE have no inner cap; they're conservatively excluded from
    the worst-case sum (their RSS is, by characterization, small enough not to be
    worth capping — the outer cap still bounds the total)."""
    return sorted((step_mem_cap_bytes(t) for t in PER_STEP_RSS_BASELINE), reverse=True)


def jobs_footprint_bytes(jobs):
    """Worst-case memory footprint (bytes) of a run at the given -j parallelism,
    from the per-step cap dict:
      * -j1  -> the LARGEST single per-step inner cap (only one step runs at once);
      * -jN  -> min( sum of the N largest per-step caps , outer cap ).
    The outer-cap clamp reflects that the outer cgroup hard-limits the total no
    matter how many steps overlap (and that the N largest caps rarely all run
    concurrently — the browser-resource serialization alone prevents it)."""
    caps = _sorted_step_caps()
    if not caps:
        return outer_cap_bytes()
    n = max(1, int(jobs))
    return min(sum(caps[:n]), outer_cap_bytes())


def jobs_for_budget(budget):
    """Inverse: the LARGEST -jN whose worst-case footprint (jobs_footprint_bytes)
    fits within `budget` bytes. Always >= 1 (if even -j1's single largest step
    cap exceeds the budget we still return 1 and let the caller surface it — a
    box too small for one step is a WAIT/abort decision, not a -j0). Capped at
    nproc (no point scheduling more parallel slots than cores). Returns
    (jobs, footprint_at_that_jobs)."""
    ncpu = os.cpu_count() or 4
    best = 1
    for n in range(1, ncpu + 1):
        if jobs_footprint_bytes(n) <= budget:
            best = n
        else:
            break  # footprint is monotonic non-decreasing in n
    return best, jobs_footprint_bytes(best)


# ---------------------------------------------------------------------------
# Memory helpers (for memory-aware parallelism + the outer-scope MemoryMax)
# ---------------------------------------------------------------------------
def parse_size(spec):
    """Parse a memory size like '8G', '4096M', '2048K', '12345' (bytes) into an
    int number of bytes. Returns None on an unparseable spec (caller decides)."""
    if not spec:
        return None
    m = re.fullmatch(r"\s*(\d+(?:\.\d+)?)\s*([KkMmGgTt]?)([Bb]?)\s*", str(spec))
    if not m:
        return None
    val = float(m.group(1))
    mult = {"": 1, "k": 1024, "m": 1024**2, "g": 1024**3, "t": 1024**4}[m.group(2).lower()]
    return int(val * mult)


def mem_available_bytes():
    """Bytes of memory currently available (MemAvailable from /proc/meminfo —
    the kernel's estimate of allocatable-without-swapping, the right basis for
    "how many more concurrent jobs fit"). None if /proc/meminfo is unreadable."""
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024  # kB -> bytes
    except (OSError, ValueError, IndexError):
        pass
    return None


def _fmt_bytes(n):
    """Human-readable size (e.g. 8.0G). Input is a byte count."""
    if n is None:
        return "?"
    v = float(n)
    for unit in ("B", "K", "M", "G"):
        if v < 1024:
            return f"{v:.0f}{unit}" if unit == "B" else f"{v:.1f}{unit}"
        v /= 1024
    return f"{v:.1f}T"

# mtg-769 / mtg-752 native-first pivot: the WASM browser e2e steps that
# EMPIRICALLY fail on the buffer-driven shadow-replay apply-frontier stall (B2;
# "ACTION COUNT MISMATCH server=N local=M" at the first bundle-window choice).
# Disabled by default (see run_orchestrator) until the foundation is hardened
# native-first and the WASM apply-frontier stall is fixed. Verified 2026-06-04 on
# a fresh wasm+server build: these four FAIL; network.click + network.redo-lobby
# PASS (buffer-driven games that don't hit the stall) so they STAY enabled, as do
# the static/asset WASM steps (landing/smoke/deploy-nav/playwright-check) and
# every NATIVE network test.
# ---------------------------------------------------------------------------
# Step registry
# ---------------------------------------------------------------------------
# Each Step: a (group, job) with a shell command, deps (other "group.job" that
# must finish first), an optional env overlay, and resource usage. The command
# runs via bash -c from PROJECT_DIR.
#
# build-once: `build.mtg-release` compiles target/release/mtg ONCE; every step
# that runs the binary reuses it (MTG_REUSE_PREBUILT=1) and lists it as a dep.
# `wasm.bundle` builds the ONE wasm-network browser bundle (mtg-dev == wasm-network
# features are identical, mtg-717 finding 8a), reused by the browser e2e, the
# equiv sweeps AND the network e2e — so there is no redundant second wasm build
# and no web/pkg clobber race (the old mtg-571 serialization is gone).
#
# resource "browser": chromium-heavy steps (browser e2e, equiv sweeps, all
# network e2e) share a capacity-limited resource so we never over-subscribe
# chromium / starve the timing-sensitive networked games. Default capacity 2
# (mtg-validate-perf-r2): two headless chromium e2e steps overlap safely — every
# networked test allocates RANDOM ports (web/test_network_*.js getRandomPorts; the
# multideck header explicitly notes "concurrent runs do not" collide) and the two
# heaviest browser steps fit well inside the outer cap (multideck 4G + gui 2G ≪
# 30G). This is the dominant validate-wall-clock lever: at capacity 1 the ~581s
# browser chain runs strictly serial and IS the critical path (700s wall @-j16,
# 45% util); capacity 2 overlaps it. Native determinism comparisons are NOT on
# this resource (they use "net", still serial — the mtg-586/589 load-sensitivity
# concern is about those, not the UI/sync browser asserts). Raise/lower with
# --browser-capacity N.

# LPT scheduling hint: when a capacity-limited resource (browser/net) frees, the
# scheduler should dispatch the LONGEST-running contended step first (classic
# longest-processing-time-first makespan heuristic) so a big step never waits
# behind a pile of small ones and ends up exposed on the tail. These are typical
# durations (seconds) from the d6dc7897/-j16 characterization run; they only
# influence DISPATCH ORDER among ready steps, never correctness, so stale numbers
# just mildly degrade packing — they are not a contract. Steps absent here sort
# after the hinted ones (hint 0), which is fine for the cheap uncontended steps.
STEP_DURATION_HINT = {
    "network.multideck": 192,
    "wasm.browser":      123,
    "network.gui":        98,
    "network.landing":    39,
    "wasm.equiv-base":    34,
    "network.redo-reload": 23,
    "network.redo-lobby": 20,
    "network.human-input": 19,
    "network.click":      19,
    "wasm.equiv-fireball": 3,
    "wasm.equiv-blackvise": 3,
    "wasm.equiv-spiritlink": 3,
}


@dataclass
class Step:
    group: str
    job: str
    desc: str
    cmd: str  # shell command (bash -c), run from PROJECT_DIR
    deps: list = field(default_factory=list)
    env: dict = field(default_factory=dict)
    resources: dict = field(default_factory=dict)  # e.g. {"browser": 1}
    networkonly: bool = False  # skipped under --no-network
    timeout: int = DEFAULT_STEP_TIMEOUT

    @property
    def tag(self):
        return f"{self.group}.{self.job}"


_REUSE = {"MTG_REUSE_PREBUILT": "1"}
_EQUIV = {"MTG_EQUIV_REQUIRE_WASM": "1", "MTG_EQUIV_NO_BUILD": "1"}
_BROWSER = {"browser": 1}
EQUIV = "./bug_finding/native_wasm_equiv_sweep.sh"


def build_registry():
    """Build the step list. The producer steps build.mtg-release + wasm.bundle
    compile the shared artifacts; in CI build-once mode (`--use-prebuilt`) they
    are DROPPED from the DAG (main()) because a gating CI job already produced
    the artifacts and the shard downloaded them — so a shard does ZERO
    compilation."""
    s = []
    add = s.append
    # --- build (once): the shared release+network mtg binary ---
    add(Step("build", "mtg-release",
             "build release+network mtg ONCE (shared by all steps)",
             "cargo build --release --bin mtg --features network",
             timeout=BUILD_STEP_TIMEOUT))
    # --- lint (self-contained; own check artifacts) ---
    add(Step("lint", "fmt", "cargo fmt --all --check", "make fmt-check"))
    add(Step("lint", "clippy", "clippy engine+benchmarks (-D warnings)", "make clippy"))
    add(Step("lint", "clippy-wasm", "clippy wasm32 target", "make clippy-wasm"))
    # beads-dupkey: a duplicate top-level frontmatter key (commonly `updated_at`,
    # from a 3-way TEXT merge of two branches that each `mb update`d the same
    # tracker) makes the YAML ambiguous and breaks `mb list`/`mb show` for the
    # WHOLE .beads dir. Fast pure-python scan; no build deps. (mtg-742 recurred
    # four times in one night before this guard.)
    add(Step("lint", "beads-dupkey",
             "reject duplicate top-level YAML keys in .beads/issues/*.md",
             "python3 scripts/check_beads_dup_keys.py .beads/issues"))
    # --- unit: `make test` (= cargo nextest run --features network). Under
    #     --use-prebuilt, CI sets NEXTEST_ARCHIVE and `make test` runs the
    #     prebuilt archive (--archive-file --workspace-remap .) instead of
    #     recompiling. The determinism/shell tests shell out to
    #     target/release/mtg via MTG_REUSE_PREBUILT (downloaded artifact).
    # deps=["build.mtg-release"]: NOT just to compile the test binaries — several
    # nextest-run tests (determinism_e2e.rs, the shell_script_tests.rs e2e wrappers)
    # SHELL OUT to the prebuilt target/release/mtg at runtime, so the release
    # binary must exist before this step. The old monolithic CI test-unit job
    # built mtg in-job, which MASKED this dep; an isolated unit shard (mtg-717
    # build-once) has no binary unless we declare it. (mtg-761)
    add(Step("unit", "nextest", "cargo nextest run --features network",
             "make test", deps=["build.mtg-release"], env=_REUSE, timeout=BUILD_STEP_TIMEOUT))
    # --- examples (own debug build) ---
    add(Step("examples", "run", "run all examples (parallel)", "make examples"))
    # --- agentplay (python; agent_game.py + mode-equivalence drive the release
    # mtg binary, so these depend on build.mtg-release. The old monolithic CI
    # test-unit job built the binary in-job, which MASKED this dep; an isolated
    # agentplay CI shard has no binary unless we declare it (mtg-717). pytest is
    # included too: its binary-needing cases skip without one, but several
    # agentplay tests spawn the engine and would otherwise fail in a fresh shard.
    add(Step("agentplay", "pytest", "pytest agentplay/", "python3 -m pytest agentplay/ -v",
             deps=["build.mtg-release"]))
    add(Step("agentplay", "mock-game", "agent_game.py mock self-play (seed 42)",
             "python3 agentplay/agent_game.py --mock --seed 42 --max-turns 5 -- "
             "decks/simple_bolt.dck decks/simple_bolt.dck; rc=$?; "
             "if [ $rc -ne 0 ] && [ $rc -ne 2 ]; then exit $rc; fi",
             deps=["build.mtg-release"]))
    add(Step("agentplay", "mode-equiv", "native/WASM mode-equivalence orchestrator",
             "./scripts/test_mode_equivalence.sh", deps=["build.mtg-release"]))
    # --- determ (full-game determinism shell scripts; reuse release mtg) ---
    add(Step("determ", "commander", "commander format E2E (full-game determinism)",
             "bash tests/commander_e2e.sh", deps=["build.mtg-release"], env=_REUSE))
    add(Step("determ", "snapshot-resume", "snapshot/resume E2E (mtg resume subcommand)",
             "bash tests/snapshot_resume_e2e.sh", deps=["build.mtg-release"], env=_REUSE))
    # --- wasm: ONE bundle (wasm-network features) + browser e2e + equiv sweeps ---
    add(Step("wasm", "bundle", "wasm-pack dev build (wasm-network) + export-wasm data",
             "make wasm-dev", timeout=BUILD_STEP_TIMEOUT))
    # OFFLINE-FIRST + NO-SILENT-SKIP provisioning (mtg-717 follow-on): use
    # vendored web/node_modules as-is if present (locked-down hosts), else
    # `npm install` with output SURFACED, else HARD-FAIL with a provisioning
    # message. ensure_node_deps.js never swallows errors and never auto-skips —
    # to run validate without the browser e2e, pass --no-wasm-e2e (reported in
    # the summary). (NPM is exported into the env so the script honors it.)
    add(Step("wasm", "npm-install", "web/ node deps (offline-first, hard-fail if absent)",
             f"cd web && NPM={NPM} {NODE} ensure_node_deps.js"))
    add(Step("wasm", "browser", "WASM browser e2e suite (19 playwright tests)",
             "cd web && " + " && ".join(
                 f"{NODE} {t}" for t in [
                     "test_fancy_tui.js", "test_bug_report.js", "test_human_input.js",
                     "test_click_and_log.js", "test_font_size_layout.js",
                     "test_decouple_step3_launch_game_session.js",
                     "test_card_size_stability.js", "test_battlefield_layout.js",
                     "test_decouple_step6_valid_choices.js", "test_tapped_rotation.js",
                     "test_graveyard_overlay.js", "test_deck_editor.js",
                     "test_deck_storage.js", "test_login_and_deck_url.js",
                     "test_cdn_image_table.js", "test_image_flicker_memo.js",
                     "test_aura_render.js", "test_render_skip.js",
                     "test_action_affordance.js"]),
             deps=["wasm.bundle", "wasm.npm-install"], resources=_BROWSER))
    for job, desc, args in [
        ("equiv-base", "native-vs-WASM STRICT sweep: old_school2/* (8 turns)",
         "--seeds 1 --decks 'decks/old_school2/*.dck' --max-turns 8"),
        ("equiv-fireball", "native-vs-WASM STRICT: multi-target Fireball (mtg-tyvcn)",
         "--seeds 1 --seed-base 15 --decks 'decks/old_school2/fireball_multitarget.dck' --max-turns 25"),
        ("equiv-blackvise", "native-vs-WASM STRICT: Black Vise ETB punisher (mtg-cuf0e)",
         "--seeds 1 --seed-base 3 --decks 'decks/old_school2/black_vise_punisher.dck' --max-turns 10"),
        ("equiv-spiritlink", "native-vs-WASM STRICT: Spirit Link non-combat lifelink (mtg-r9po1)",
         "--seeds 1 --seed-base 26 --decks 'decks/old_school2/spirit_link_pinger.dck' --max-turns 16"),
    ]:
        add(Step("wasm", job, desc, f"{EQUIV} {args}",
                 deps=["build.mtg-release", "wasm.bundle"], env=_EQUIV, resources=_BROWSER))
    # --- network e2e ---
    # BROWSER-net: spawn `mtg server` + a headless chromium client over the
    # wasm-network bundle. Need build.mtg-release + the wasm bundle + npm;
    # browser-resource-serialized. (CI sub-shards: network-gui, network-redo.)
    net_browser = [
        # Deps are provisioned by wasm.npm-install (a dep of every net_browser
        # step, below) — this step just VERIFIES chromium is present and
        # hard-fails with an actionable message if not. No more swallowing
        # `npm install ... 2>/dev/null || true` (which hid the real reason).
        ("playwright-check", "verify playwright chromium provisioned",
         f"cd web && {NODE} playwright_check.js"),
        ("gui", "networked GUI e2e (baseline)", "cd web && node test_network_gui_e2e.js"),
        ("human-input", "networked HUMAN-controller sync gate (mtg-679)", "cd web && node test_network_human_input.js"),
        ("multideck", "networked multi-deck e2e (--quick)", "cd web && node test_network_multideck.js --quick"),
        ("click", "networked click+log e2e", "cd web && node test_network_click.js"),
        ("landing", "landing-page UX e2e", "cd web && node test_landing_page_ux.js"),
        ("redo-reload", "lobby-redo multiturn + mid-game reload (mtg-682 4+5)", "cd web && node test_redo_multiturn_reload_e2e.js"),
        ("redo-lobby", "lobby-flow-fixes e2e (mtg-682 1-4)", "cd web && node test_redo_lobby_e2e.js"),
        ("players-list", "lobby logged-in players list e2e (mtg-890)", "cd web && node test_lobby_players_list_e2e.js"),
        ("smoke", "hermetic CAS web-asset smoke (mtg-571)", "cd web && node test_web_server_smoke.js"),
        ("deploy-nav", "hashed deploy-tree navigation gate (mtg-682)", "cd web && node test_deploy_tree_nav.js"),
    ]
    for job, desc, cmd in net_browser:
        add(Step("network", job, desc, cmd,
                 deps=["build.mtg-release", "wasm.bundle", "wasm.npm-install"],
                 resources=_BROWSER, networkonly=True))
    # NATIVE-net: `mtg server` + `mtg connect` only — NO browser, NO wasm bundle
    # (verified: these scripts never reference chromium/playwright/web/pkg). So
    # they depend ONLY on build.mtg-release and use a "net" resource (serialized
    # among themselves to keep the determinism comparisons load-stable, per the
    # mtg-586/589 load-sensitivity note) — NOT the browser resource. (CI
    # sub-shard: network-equiv, which needs neither node nor wasm-pack.)
    net_native = [
        ("equiv-random", "network-vs-local gamelog identity: random (mtg-380)", "bash tests/network_vs_local_equivalence_e2e.sh 3 random", {}),
        ("equiv-zero", "network-vs-local gamelog identity: zero", "bash tests/network_vs_local_equivalence_e2e.sh 3 zero", {}),
        ("equiv-heuristic", "network-vs-local gamelog identity: heuristic (mtg-yulth)", "bash tests/network_vs_local_equivalence_e2e.sh 3 heuristic", {}),
        ("robots42", "robots42 state-sync regression (mtg-559)", "bash tests/robots42_state_sync_e2e.sh", {}),
        ("fuzz", "bounded determinism + net-equiv fuzz", "bash tests/fuzz_determinism_netequiv_e2e.sh", _REUSE),
    ]
    for job, desc, cmd, env in net_native:
        add(Step("network", job, desc, cmd, deps=["build.mtg-release"],
                 env=dict(env), resources={"net": 1}, networkonly=True))
    return s


# ---------------------------------------------------------------------------
# Per-step summary extraction (the one-line default summary)
# ---------------------------------------------------------------------------
def summarize(step, detail_path):
    """Return a short human summary of a step's result from its detail log."""
    try:
        text = detail_path.read_text(errors="replace")
    except OSError:
        return ""
    # nextest: "Summary [   ...] 1136 tests run: 1136 passed, 0 skipped"
    m = re.search(r"(\d+) tests? run: (\d+) passed.*?(\d+) skipped", text)
    if m:
        return f"{m.group(2)} passed, {m.group(3)} skipped"
    # pytest: "=== N passed in Ms ==="
    m = re.search(r"(\d+) passed(?:, (\d+) skipped)?", text)
    if m and "pytest" in step.cmd:
        sk = f", {m.group(2)} skipped" if m.group(2) else ""
        return f"{m.group(1)} passed{sk}"
    # equiv sweeps: "0 diverged" / "N/N"
    m = re.search(r"(\d+)\s+diverged", text)
    if m:
        return f"{m.group(1)} diverged"
    # examples: "N examples ... PASSED" — count
    m = re.search(r"(\d+)\s+passed.*\b(\d+)\s+failed", text)
    if m:
        return f"{m.group(1)} passed, {m.group(2)} failed"
    return ""


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------
class Runner:
    def __init__(self, steps, jobs, verbosity, steps_dir, resource_caps, disabled=None,
                 keep_going=False, cgroups=None):
        # Per-step cgroup manager (validate_cgroup.StepCgroups) or None. When
        # enabled, each step runs in its OWN child cgroup under the outer scope
        # and is torn down via cgroup.kill — which catches setsid/double-fork
        # escapees that killpg cannot. killpg stays as the fallback + belt-and-
        # suspenders (we do BOTH on teardown when cgroups are enabled).
        self.cgroups = cgroups
        self.disabled = dict(disabled or {})  # tag -> flag reason (explicit opt-out)
        # EAGER-EXIT (default): on the FIRST step failure, kill the steps still
        # running in parallel (via their process groups) and stop immediately,
        # instead of letting the in-flight wave finish. --keep-going flips this
        # to "run everything, report all failures in one pass".
        self.keep_going = keep_going
        self.running_pgids = {}  # tag -> pgid of an in-flight step (for eager kill)
        self.aborted = set()     # tags killed by eager-exit (labelled, not FAIL)
        self.steps = {s.tag: s for s in steps}
        # Dispatch order = LONGEST-contended-step-first (LPT makespan heuristic):
        # sort by the static duration hint descending so that when a scarce
        # resource (browser/net) frees, the heaviest ready step claims it instead
        # of a short one — keeping the big steps off the critical-path tail. The
        # sort is STABLE, so steps with equal/no hint keep registration order
        # (deps are still enforced separately in run(); order only affects which
        # READY step is picked first). See STEP_DURATION_HINT.
        self.order = sorted(
            (s.tag for s in steps),
            key=lambda tag: STEP_DURATION_HINT.get(tag, 0),
            reverse=True,
        )
        self.jobs = jobs
        self.verbosity = verbosity  # 0 quiet, 1 default(+summary), 2 stream-framework, 3 stream-all
        self.steps_dir = steps_dir
        self.resource_avail = dict(resource_caps)
        self.lock = threading.Lock()
        self.done = {}          # tag -> (ok, duration, summary)
        self.running = set()
        self.failed = False
        self.stop = False       # stop scheduling new steps after a failure
        steps_dir.mkdir(parents=True, exist_ok=True)

    def _emit(self, line):
        with self.lock:
            sys.stdout.write(line + "\n")
            sys.stdout.flush()

    def _deps_ok(self, step):
        return all(self.done.get(d, (False,))[0] for d in step.deps)

    def _deps_known(self, step):
        # a dep that FAILED makes this step unrunnable (skip)
        return all(d in self.done for d in step.deps)

    def _res_free(self, step):
        return all(self.resource_avail.get(r, 0) >= n for r, n in step.resources.items())

    def _acquire(self, step):
        for r, n in step.resources.items():
            self.resource_avail[r] -= n

    def _release(self, step):
        for r, n in step.resources.items():
            self.resource_avail[r] += n

    def run(self):
        threads = []
        wall_start = time.time()
        while True:
            with self.lock:
                # done?
                # Terminate when nothing is running AND either every step has a
                # terminal outcome (done or dep-skipped) OR fail-fast tripped.
                # mtg-717: the fail-fast clause is REQUIRED. When self.stop is set
                # by a failed step, the remaining steps whose deps SUCCEEDED (e.g.
                # the other browser steps in the network-redo shard, all depending
                # on the prebuilt build.mtg-release/wasm.bundle) are neither
                # launched (stop blocks launch) nor counted by _skipped() (no dep
                # failed). Without this clause `done+skipped >= steps` is never
                # reached, so the loop busy-waits forever — that was the network-
                # redo 56-min CI wedge (the orphan python3 pid in the cleanup
                # trace was this livelocked runner, NOT a pipe-blocked reader).
                if not self.running and (
                    self.stop
                    or len(self.done) + len(self._skipped()) >= len(self.steps)
                ):
                    break
                launchable = []
                if not self.stop:
                    for tag in self.order:
                        if tag in self.done or tag in self.running or tag in self._skipped():
                            continue
                        step = self.steps[tag]
                        if not self._deps_known(step):
                            continue  # deps not resolved yet
                        if not self._deps_ok(step):
                            continue  # a dep failed -> handled as skip in _skipped()
                        if len(self.running) >= self.jobs:
                            break
                        if not self._res_free(step):
                            continue
                        launchable.append(step)
                        self.running.add(tag)
                        self._acquire(step)
                for step in launchable:
                    t = threading.Thread(target=self._run_step, args=(step,), daemon=True)
                    t.start()
                    threads.append(t)
            time.sleep(0.05)
        for t in threads:
            t.join()
        self.wall = time.time() - wall_start
        return not self.failed

    def _skipped(self):
        # steps whose deps failed -> never run
        sk = set()
        changed = True
        while changed:
            changed = False
            for tag, step in self.steps.items():
                if tag in sk or tag in self.done or tag in self.running:
                    continue
                for d in step.deps:
                    if (d in self.done and not self.done[d][0]) or d in sk:
                        sk.add(tag)
                        changed = True
                        break
        return sk

    def _reap(self, pgid, tag=None):
        """Tear down a step's whole process tree. When per-step cgroups are
        enabled, `cgroup.kill` the step's child cgroup FIRST — that SIGKILLs the
        ENTIRE subtree atomically, including setsid/double-fork escapees that a
        process-group kill misses (an escapee changes session/pgid but NOT
        cgroup membership). Then ALSO killpg as a belt-and-suspenders for the
        no-cgroup path (and harmless when the cgroup already cleared it).

        The pgid is captured right after Popen — NOT os.getpgid(proc.pid), which
        fails once proc.wait() has reaped the leader (the group still exists
        while any grandchild lives, so the stored pgid stays valid). Scoped to
        the step's own session via start_new_session; guarded so we never signal
        the runner's own group (suicide / the historical exit-144)."""
        if self.cgroups is not None and tag is not None:
            self.cgroups.kill(tag)
        if pgid is None or pgid <= 1 or pgid == os.getpgrp():
            return
        try:
            os.killpg(pgid, signal.SIGKILL)
        except (ProcessLookupError, OSError):
            pass  # whole group already gone

    def _run_step(self, step):
        self._emit(f"[{step.tag}] ▶ START  {step.desc}")
        detail = self.steps_dir / f"{step.tag}.log"
        env = dict(os.environ)
        env.update(step.env)
        start = time.time()
        stream = self.verbosity >= 2
        timed_out = False
        fh = open(detail, "wb")
        # Network/browser tests spawn GRANDCHILDREN (mtg server, python
        # http.server, chromium). An orphan that survives the test holds the
        # stdout pipe's write-end open, so a naive `for raw in proc.stdout` on
        # the main thread would never see EOF and hang forever (this hung the
        # wasm/network CI shards for ~73min). So we read on a DAEMON thread —
        # the main thread blocks only on proc.wait(timeout), which returns when
        # the test process itself exits regardless of orphans — and bound each
        # step with a wall-clock timeout.
        # start_new_session=True puts the step in its OWN process group/session
        # (pgid == child pid), so we can reap the WHOLE tree (bash leader + mtg
        # server + http.server + chromium) by killing that group on completion —
        # WITHOUT ever touching the runner's own group. mtg-743: a failed/hung
        # browser test used to leak chromium/server orphans that held resources
        # (and the stdout pipe) into later steps; scoped-killpg reaps them. The
        # historical exit-144 came from killpg WITHOUT start_new_session (the
        # child shared the runner's pgid, so killpg was suicide) — the new
        # session makes it safe; _reap() additionally guards against our pgrp.
        # When per-step cgroups are enabled, wrap the command so the step's bash
        # leader self-moves into its own child cgroup as its FIRST action — every
        # grandchild it then forks inherits that cgroup (cgroup v2 fork rule), so
        # cgroup.kill later reaps the whole tree incl. setsid escapees.
        run_cmd = step.cmd
        if self.cgroups is not None:
            # Per-step INNER cgroup MemoryMax from the characterized baseline (or
            # None for an un-characterized / deliberately-excluded step like
            # determ.commander — then only the OUTER cap protects it). mtg-887.
            run_cmd = self.cgroups.prepare_command(
                step.tag, step.cmd, mem_max=step_mem_cap_bytes(step.tag))
        proc = subprocess.Popen(["bash", "-c", run_cmd], cwd=PROJECT_DIR, env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                start_new_session=True)
        # Capture the group id NOW, while the leader is alive. start_new_session
        # makes pgid == proc.pid; after proc.wait() collects the leader,
        # os.getpgid(proc.pid) would raise — but killpg(pgid) still reaps any
        # surviving grandchildren, so we stash it for _reap().
        try:
            step_pgid = os.getpgid(proc.pid)
        except (ProcessLookupError, OSError):
            step_pgid = proc.pid
        # Register the pgid so a sibling that FAILS can eager-kill this in-flight
        # step (see the failure branch below).
        with self.lock:
            self.running_pgids[step.tag] = step_pgid

        def _pump():
            try:
                for raw in proc.stdout:
                    fh.write(raw)
                    if stream:
                        self._emit(f"[{step.tag}] " + raw.decode(errors="replace").rstrip("\n"))
            except Exception:
                pass

        reader = threading.Thread(target=_pump, daemon=True)
        reader.start()
        try:
            proc.wait(timeout=step.timeout)
        except subprocess.TimeoutExpired:
            # Genuine hang: reap the step's whole process group (bash leader +
            # any server/chromium grandchildren). Safe because start_new_session
            # gave the step its own group (pgid == child pid); _reap guards
            # against signalling the runner's own group. The unconditional
            # _reap() below also covers the non-timeout path.
            timed_out = True
            self._reap(step_pgid, step.tag)
            try:
                proc.wait(timeout=10)
            except Exception:
                pass
        # The daemon reader may still be blocked on an orphan-held pipe (a test
        # left a server/chromium holding stdout). That is exactly the old hang —
        # but because the reader is a DAEMON and we don't block on it, the step
        # completes regardless. proc.wait() already returned once the test
        # process itself exited; orphans don't gate us. Brief join, then move on.
        reader.join(timeout=2)
        # mtg-743: reap the step's whole process group, so any orphan
        # grandchildren (mtg server / http.server / chromium that outlived the
        # test) are SIGKILLed now instead of leaking into later steps or holding
        # the stdout pipe. Scoped to the step's own session (start_new_session
        # above) — never the runner's group. This also lets the abandoned reader
        # thread finally see EOF.
        self._reap(step_pgid, step.tag)
        # Capture OOM + peak from the step's cgroup BEFORE cleanup() rmdirs it.
        step_oom = 0
        step_peak = None
        if self.cgroups is not None:
            step_oom = self.cgroups.oom_kills(step.tag)
            step_peak = self.cgroups.peak_bytes(step.tag)
            self.cgroups.cleanup(step.tag)  # rmdir the now-empty child cgroup
        try:
            fh.close()
        except Exception:
            pass
        dur = round(time.time() - start)
        ok = (proc.returncode == 0) and not timed_out
        if step_peak is not None:
            # Record per-step peak RSS into the detail (baseline characterization
            # — compare against PER_STEP_RSS_BASELINE to retune caps; mtg-887).
            try:
                with open(detail, "ab") as f2:
                    cap = step_mem_cap_bytes(step.tag)
                    capnote = f" (inner cap {_fmt_bytes(cap)})" if cap else " (no inner cap)"
                    f2.write(f"\n[validate_run] step peak RSS: {_fmt_bytes(step_peak)}{capnote}\n".encode())
            except OSError:
                pass
        if timed_out:
            with open(detail, "ab") as f2:
                f2.write(f"\n[validate_run] STEP TIMED OUT after {step.timeout}s — SIGKILLed process group\n".encode())
        summary = summarize(step, detail)
        with self.lock:
            self.running.discard(step.tag)
            self.running_pgids.pop(step.tag, None)
            self._release(step)
            was_aborted = step.tag in self.aborted
            self.done[step.tag] = (False if was_aborted else ok, dur, summary)
            if not was_aborted and not ok:
                # A REAL failure. Mark failed + stop scheduling new steps. EAGER-EXIT
                # (default): also kill every step still running in parallel NOW, so a
                # fast failure doesn't wait for a slow in-flight build to finish.
                # --keep-going leaves them running (collect all failures in one pass).
                self.failed = True
                self.stop = True
                if not self.keep_going:
                    for other, pgid in list(self.running_pgids.items()):
                        self.aborted.add(other)   # so its thread labels itself ABORTED, not FAIL
                        self._reap(pgid, other)    # cgroup.kill + killpg -> its proc.wait returns
        if was_aborted:
            self._emit(f"[{step.tag}] ⊘ ABORT  {step.desc} ({dur}s — eager-exit after another step failed; --keep-going to run all)")
        elif ok:
            extra = f"  [{summary}]" if (summary and self.verbosity >= 1) else ""
            self._emit(f"[{step.tag}] ✓ PASS   {step.desc} ({dur}s){extra}")
        else:
            oomed = step_oom > 0
            if oomed:
                why = f"OOM-KILLED (hit inner MemoryMax; {step_oom} oom_kill event(s))"
            elif timed_out:
                why = f"TIMEOUT >{step.timeout}s"
            else:
                why = f"exit {proc.returncode}"
            self._emit(f"[{step.tag}] ✗ FAIL   {step.desc} ({dur}s, {why})")
            if oomed:
                # ACTIONABLE OOM message (mtg-887 item 4): (a) which step, (b)
                # WHERE the baseline lives, (c) how to SAFELY raise it.
                cap = step_mem_cap_bytes(step.tag)
                peak = _fmt_bytes(step_peak) if step_peak else "?"
                self._emit(f"[{step.tag}] ▲ MEMORY CAP HIT: step '{step.tag}' was OOM-killed at its "
                           f"inner cgroup MemoryMax (cap≈{_fmt_bytes(cap)}, peak≈{peak}).")
                self._emit(f"[{step.tag}] ▲   BASELINE: scripts/validate.py → PER_STEP_RSS_BASELINE['{step.tag}'] "
                           f"(x MEM_CAP_FACTOR={MEM_CAP_FACTOR}).")
                self._emit(f"[{step.tag}] ▲   BEFORE RAISING IT: confirm this is GENUINE growth, not an "
                           f"UNBOUNDED LEAK/loop (re-run; check peak grows with input size, not without "
                           f"bound). A cap bump without that check just re-arms the OOM gun. Then bump "
                           f"PER_STEP_RSS_BASELINE['{step.tag}'] to the new typical peak (keep the 1.25x "
                           f"factor) — or set VALIDATE_MEM_CAP_FACTOR=1.5 if the 1.25x is too tight.")
            # self-contained failure: dump tagged detail into the log
            self._emit(f"[{step.tag}] ----- detail ({detail}) -----")
            try:
                for ln in detail.read_text(errors="replace").splitlines():
                    self._emit(f"[{step.tag}] {ln}")
            except OSError:
                pass
            self._emit(f"[{step.tag}] ----- end detail -----")

    # -- stats --
    def print_stats(self):
        results = [(tag, *self.done[tag]) for tag in self.order if tag in self.done]
        serial = sum(d for _, _, d, _ in results)
        speedup = (serial / self.wall) if self.wall > 0 else 0.0
        skipped = sorted(self._skipped())
        print("")
        print("=" * 60)
        print("VALIDATE STATS")
        print("=" * 60)
        print(f"  wall-clock:      {self.wall:6.0f}s")
        print(f"  serial-sum:      {serial:6.0f}s  (sum of all step durations)")
        print(f"  parallel speedup: {speedup:5.2f}x  on -j{self.jobs}")
        # per-group
        groups = {}
        for tag, ok, dur, _ in results:
            groups.setdefault(tag.split(".")[0], 0)
            groups[tag.split(".")[0]] += dur
        print("  per-group serial time:")
        for g, t in sorted(groups.items(), key=lambda kv: -kv[1]):
            print(f"      {g:12s} {t:5.0f}s")
        # slowest steps (critical-path candidates)
        print("  slowest steps:")
        for tag, ok, dur, _ in sorted(results, key=lambda r: -r[2])[:6]:
            print(f"      {dur:5.0f}s  {tag}")
        if skipped:
            print(f"  SKIPPED (dep failed): {', '.join(skipped)}")
        aborted = sorted(t for t in self.aborted if t in self.done)
        if aborted:
            print(f"  ABORTED (eager-exit, not run to completion): {', '.join(aborted)}")
        # Explicitly-disabled steps (opt-out flags) are REPORTED, never hidden —
        # a flagged run must not be mistaken for full coverage.
        if self.disabled:
            by_reason = {}
            for tag, reason in sorted(self.disabled.items()):
                by_reason.setdefault(reason, []).append(tag)
            for reason, tags in sorted(by_reason.items()):
                print(f"  DISABLED via {reason} ({len(tags)}): {', '.join(tags)}")
        npass = sum(1 for tag, ok, _, _ in results if ok)
        naborted = sum(1 for tag, ok, _, _ in results if (not ok) and tag in self.aborted)
        nfail = sum(1 for tag, ok, _, _ in results if (not ok) and tag not in self.aborted)
        ndis = len(self.disabled)
        dis_note = f", {ndis} DISABLED (explicit flag — NOT full coverage)" if ndis else ""
        ab_note = f", {naborted} aborted (eager-exit)" if naborted else ""
        print(f"  result: {npass} passed, {nfail} failed, {len(skipped)} skipped{ab_note}{dis_note}")
        print("=" * 60)


def _print_mem_footprint(args):
    """--query-mem-footprint: report the memory model (does NOT run validate).
    Everything derives from the constants in scripts/validate.py (single source
    of truth): outer cap + the per-step cap dict."""
    ncpu = os.cpu_count() or 4
    j = args.jobs if args.jobs is not None else ncpu
    avail = mem_available_bytes()
    total = _total_ram_bytes()
    print("=== validate memory footprint (from scripts/validate.py constants) ===")
    print(f"  whole-run outer cap : {_fmt_bytes(outer_cap_bytes())} "
          f"(VALIDATE_TOTAL_RSS_BASELINE_BYTES {_fmt_bytes(VALIDATE_TOTAL_RSS_BASELINE_BYTES)} "
          f"× MEM_CAP_FACTOR {MEM_CAP_FACTOR})")
    print(f"  per-step inner caps : {', '.join(_fmt_bytes(c) for c in _sorted_step_caps())} "
          f"(descending; {len(PER_STEP_RSS_BASELINE)} characterized steps)")
    print(f"  this host           : {_fmt_bytes(total)} RAM, {_fmt_bytes(avail)} available now, "
          f"{ncpu} cores")
    print(f"\n  footprint at -j{j} (current): {_fmt_bytes(jobs_footprint_bytes(j))}")
    print("\n  footprint by -j (worst case = min(sum of N largest step caps, outer cap)):")
    shown = sorted(set([1, 2, 3, 4, 5, 8, ncpu]))
    for n in shown:
        if n <= ncpu:
            print(f"      -j{n:<3d} {_fmt_bytes(jobs_footprint_bytes(n))}")
    print("\n  largest -j that fits a budget (what --max-mem M would pick):")
    for spec in ("8G", "16G", "24G", "32G", "48G", "64G"):
        b = parse_size(spec)
        jb, fp = jobs_for_budget(b)
        fits = "✓" if jobs_footprint_bytes(jb) <= b else "⚠ even -j1 over budget"
        print(f"      --max-mem {spec:<4s} → -j{jb:<3d} (footprint {_fmt_bytes(fp)}) {fits}")
    if avail:
        rec_budget = int(avail * 0.8)
        jb, fp = jobs_for_budget(rec_budget)
        print(f"\n  greedy recommendation for THIS host (avail×0.8 = {_fmt_bytes(rec_budget)}):")
        if jobs_footprint_bytes(1) > rec_budget:
            print(f"      WAIT — even -j1 needs {_fmt_bytes(jobs_footprint_bytes(1))} > "
                  f"{_fmt_bytes(rec_budget)}; let running validates finish first.")
        elif jb >= ncpu:
            print(f"      run FULL -j{ncpu} (ample headroom; no --max-mem needed).")
        else:
            print(f"      run with --max-mem {_fmt_bytes(rec_budget)} → -j{jb} "
                  f"(footprint {_fmt_bytes(fp)} fits).")


def _resolve_jobs(args):
    """Resolve args.jobs in place. Precedence: --sequential (1) > explicit -j >
    --max-mem-derived -j > nproc.

    -j is derived BACKWARDS from the per-step cap dict (jobs_for_budget): the
    largest -jN whose worst-case footprint fits the budget. This replaces the old
    flat `budget / --mem-per-job` estimate (which ignored the real per-step
    sizes). It only ever REDUCES below nproc when the budget is genuinely tight —
    the DEFAULT outer cap clamps the footprint to itself, so jobs_for_budget
    returns nproc there (no throttle on an ample box); only an explicit, tighter
    --max-mem shrinks -j. If even -j1's footprint exceeds the budget we keep -j1
    and WARN (the box is too small for one step — a WAIT/abort decision)."""
    if args.sequential:
        args.jobs = 1
        return
    if args.jobs is not None:
        return  # an explicit -j always wins, even under --max-mem
    ncpu = os.cpu_count() or 4
    budget = _resolve_mem_budget(args)
    if not budget:
        args.jobs = ncpu  # --no-max-mem: uncapped, full parallelism
        return
    jobs, fp = jobs_for_budget(budget)
    explicit_budget = bool(getattr(args, "max_mem", None))
    over_budget = jobs == 1 and jobs_footprint_bytes(1) > budget
    if jobs < ncpu and explicit_budget and not over_budget:
        print(f"[validate] --max-mem {_fmt_bytes(budget)} → -j{jobs}: worst-case footprint "
              f"{_fmt_bytes(fp)} ≤ {_fmt_bytes(budget)} (largest -jN that fits, from the "
              f"per-step cap dict).")
    if over_budget:
        print(f"[validate] ⚠ --max-mem {_fmt_bytes(budget)}: even -j1 needs "
              f"{_fmt_bytes(jobs_footprint_bytes(1))} — the largest single step won't fit the "
              f"budget. Running -j1 anyway; its inner cap may OOM-kill it. Consider WAITING for "
              f"other validates to free RAM.")
    args.jobs = jobs


def main():
    ap = argparse.ArgumentParser(description="make validate orchestrator (mtg-717)")
    ap.add_argument("--jobs", "-j", type=int, default=None,
                    help="max parallel steps (default: number of CPUs, or auto-capped by "
                         "--max-mem). An explicit -j always wins over memory auto-scaling.")
    ap.add_argument("--group", help="comma-separated jobGroups to run (CI shard)")
    ap.add_argument("--only", help="comma-separated group.job to run (+their deps)")
    ap.add_argument("--job", help="comma-separated jobIds to run (any group)")
    ap.add_argument("--no-network", action="store_true",
                    help="DELIBERATELY disable the network jobGroup (reported in the run summary; "
                         "never a silent skip)")
    ap.add_argument("--no-wasm-e2e", "--no-browser", dest="no_wasm_e2e", action="store_true",
                    help="DELIBERATELY disable all browser/chromium e2e steps (wasm browser suite, "
                         "native-vs-WASM equiv sweeps, networked browser e2e, + their npm provisioning). "
                         "Use on a host without a usable browser/npm. Disabled steps are REPORTED in the "
                         "run summary so a flagged run is never mistaken for full coverage.")
    ap.add_argument("--enable-wasm-network", action="store_true",
                    help="DEPRECATED NO-OP. The WASM network-game e2e steps "
                         "(gui/multideck/human-input/redo-*) are now ENABLED BY DEFAULT "
                         "(mtg-752: the WASM reorder/reveal split + apply-frontier fix landed). "
                         "Flag retained so existing invocations do not error; use --no-wasm-e2e to "
                         "disable all browser/chromium steps on a host without a usable browser.")
    ap.add_argument("--browser-capacity", type=int, default=2,
                    help="how many chromium-heavy steps may run at once (default 2 — the ~581s "
                         "browser e2e chain is the validate critical path at capacity 1; two "
                         "headless chromium steps overlap safely on random ports, see the "
                         "STEP_DURATION_HINT / 'resource browser' note. Set 1 for the old "
                         "strictly-serial behaviour.)")
    ap.add_argument("--net-capacity", type=int, default=1,
                    help="how many native networked-game steps may run at once (default 1)")
    ap.add_argument("--use-prebuilt", action="store_true",
                    help="CI build-once mode: a gating job already compiled the shared artifacts "
                         "(release mtg, wasm bundle, nextest archive) and this shard downloaded "
                         "them. DROP the build.mtg-release + wasm.bundle compile steps from the DAG "
                         "(and strip them from every step's deps) so the shard does ZERO "
                         "compilation. Tests reuse the downloaded artifacts via MTG_REUSE_PREBUILT "
                         "/ MTG_EQUIV_NO_BUILD / NEXTEST_ARCHIVE (set by CI). HARD-FAILS if an "
                         "expected artifact is missing — never silently cold-rebuilds.")
    ap.add_argument("-q", "--quiet", action="store_true")
    ap.add_argument("-v", dest="v", action="count", default=0, help="-v stream framework, -vv stream all")
    ap.add_argument("--list", action="store_true", help="list steps and exit")
    ap.add_argument("--dot", action="store_true",
                    help="emit the step DAG as graphviz (dep edges + dashed resource-"
                         "serialization edges) and exit; honors --group/--only/--no-network/"
                         "--no-wasm-e2e/--use-prebuilt so the graph matches what WOULD run. "
                         "Pipe to: python3 scripts/validate.py --dot | dot -Tsvg -o validate.svg")
    # --- outer harness flags (folded in from the former scripts/validate.sh) ---
    # The harness (commit-hash cache, .validate.lock, dirty->WIP-commit, clean-env
    # gate, CPU-utilization report, atomic validate_logs/validate_<sha>.log
    # artifact + latest symlink) runs ONLY for a FULL local `make validate`. Any
    # subset run (--group/--only/--job — i.e. every CI shard + focused local runs),
    # --list, or --use-prebuilt skips the harness and runs the orchestrator
    # directly, preserving CI's hermetic behaviour (no cache/lock/WIP in CI).
    ap.add_argument("--force", action="store_true",
                    help="harness: bypass the commit-hash cache and always run")
    ap.add_argument("--sequential", action="store_true",
                    help="harness: run sequentially (-j1) for easier debugging")
    ap.add_argument("--no-wip-commit", action="store_true",
                    help="harness: don't create a temporary WIP commit if the tree is dirty "
                         "(runs anyway; caching disabled for the run)")
    ap.add_argument("--force-wip-commit", action="store_true",
                    help="harness: include submodule changes in the WIP commit (else dirty "
                         "submodules abort)")
    ap.add_argument("--no-monitor-utilization", action="store_true",
                    help="harness: disable the background CPU-utilization sampler/report")
    ap.add_argument("--no-harness", action="store_true",
                    help="run the orchestrator directly with NO outer harness (no cache/lock/"
                         "WIP/log-artifact) — what CI shards effectively do")
    ap.add_argument("--keep-going", action="store_true",
                    help="on a step failure, KEEP RUNNING the rest (collect every failure in one "
                         "pass) instead of the default EAGER-EXIT (kill in-flight steps + stop at "
                         "the first failure for fast feedback)")
    ap.add_argument("--no-scope", action="store_true",
                    help="do NOT re-exec a full local validate inside a transient systemd --user "
                         "scope (the default self-isolation that reaps ALL descendants — incl. "
                         "setsid escapees — on exit). Use if systemd-run misbehaves.")
    ap.add_argument("--no-step-cgroups", action="store_true",
                    help="do NOT give each step its OWN child cgroup under the outer scope "
                         "(per-step cgroup.kill is setsid-proof where killpg is not). Falls back "
                         "to the killpg-only reaper. No-op when the outer scope is unavailable.")
    ap.add_argument("--max-mem", metavar="SPEC",
                    help="OVERRIDE the DEFAULT memory cap on the whole validate run (the outer "
                         "scope's MemoryHigh/MemoryMax). A cap is applied BY DEFAULT (1.25x the "
                         "characterized whole-run peak; see VALIDATE_TOTAL_RSS_BASELINE_BYTES) so a "
                         "runaway can never OOM the box; use this only to raise/lower it. SPEC is an "
                         "absolute size (e.g. 8G, 4096M) applied as MemoryMax (MemoryHigh=90%% of it), "
                         "or 'auto' to budget from /proc/meminfo MemAvailable. Unless -j was given "
                         "explicitly, -j is then derived BACKWARDS from the per-step cap dict: the "
                         "largest -jN whose worst-case footprint (sum of the N largest per-step caps, "
                         "clamped at the outer cap) fits in SPEC. The computation is printed.")
    ap.add_argument("--no-max-mem", action="store_true",
                    help="DANGEROUS opt-out: run with NO outer memory cap. A runaway test can then "
                         "OOM the whole host (this is exactly the 2026-06-09 box-wedge). Only for "
                         "deliberate memory profiling where the cap distorts the measurement.")
    ap.add_argument("--query-mem-footprint", action="store_true",
                    help="QUERY (does NOT run validate): print the worst-case memory footprint a run "
                         "would use at the current -j (from the per-step cap dict — -j1 = the largest "
                         "single per-step cap; -jN = min(sum of N largest caps, outer cap)), plus the "
                         "footprint at every -jN and which budgets pick which -j, then exit. Use to "
                         "decide a safe -j / --max-mem before launching.")
    args = ap.parse_args()

    if args.query_mem_footprint:
        _print_mem_footprint(args)
        return 0

    _resolve_jobs(args)
    subset = bool(args.group or args.only or args.job)
    use_harness = (not subset) and (not args.list) and (not args.dot) \
        and (not args.no_harness) and (not args.use_prebuilt)
    if use_harness:
        return run_with_harness(args)
    return run_orchestrator(args)


def _emit_dot(steps):
    """Emit the (already-filtered) step DAG as graphviz DOT.
    - SOLID edges = explicit deps (build-once falls out of these).
    - DASHED edges = implicit serialization among steps sharing a capacity-1
      resource (e.g. the cap-1 'browser' resource → chromium-heavy steps run
      ONE-AT-A-TIME with NO dep edge between them; a pure dep graph understates
      the real ordering). The dashed chain is drawn in registry order purely to
      VISUALISE the constraint — actual order is scheduler-chosen.
    Steps are clustered per jobGroup. Honors whatever filtering already ran
    (subset / --no-network / --no-wasm-e2e / --use-prebuilt), so the emitted
    graph matches what WOULD actually run in that mode."""
    tags = {s.tag for s in steps}
    out = ['digraph validate {',
           '  rankdir=LR;',
           '  node [shape=box, style=rounded, fontsize=10];',
           '  labelloc="t";',
           '  label="make validate DAG  (solid = dependency,  dashed = shared cap-1 '
           'resource → serialized, order scheduler-chosen)";']
    groups = {}
    for s in steps:
        groups.setdefault(s.group, []).append(s)
    for gi, (g, gsteps) in enumerate(sorted(groups.items())):
        out.append(f'  subgraph cluster_{gi} {{')
        out.append(f'    label="{g}"; style=dashed; color=gray70;')
        for s in gsteps:
            res = ("\\n[" + ",".join(sorted(s.resources)) + "]") if s.resources else ""
            out.append(f'    "{s.tag}" [label="{s.tag}{res}"];')
        out.append('  }')
    for s in steps:                       # solid dep edges (present deps only)
        for d in s.deps:
            if d in tags:
                out.append(f'  "{d}" -> "{s.tag}";')
    res_members = {}                      # dashed resource-serialization chains
    for s in steps:
        for r in s.resources:
            res_members.setdefault(r, []).append(s.tag)
    colors = ["red", "blue", "darkgreen", "purple"]
    for ri, (r, members) in enumerate(sorted(res_members.items())):
        if len(members) < 2:
            continue
        c = colors[ri % len(colors)]
        for a, b in zip(members, members[1:]):
            out.append(f'  "{a}" -> "{b}" [style=dashed, color={c}, constraint=false, '
                       f'label="{r} cap1", fontsize=8];')
    out.append('}')
    print("\n".join(out))


def run_orchestrator(args):
    """Build the step DAG (honoring filters/flags) and run it. No outer harness
    (cache/lock/WIP/log-artifact) — that lives in run_with_harness()."""
    steps = build_registry()
    # Explicit opt-out flags DISABLE steps — but we RECORD what was disabled (tag
    # -> reason) and report it in the run summary, so a flagged run is never
    # silently mistaken for full coverage (the never-skip principle: a disabled
    # step must be visible, not vanish).
    disabled = {}
    if args.no_network:
        for s in steps:
            if s.networkonly:
                disabled[s.tag] = "--no-network"
    # mtg-769 / mtg-752: the WASM network-GAME e2e steps were temporarily
    # disabled-by-default while the buffer-driven shadow-replay foundation was
    # hardened on NATIVE first. That work has landed (native buffer shim) AND the
    # WASM client now has the reorder/reveal split + eager-reveal apply-frontier
    # (B2) fix, so these steps are RE-ENABLED and run by default again. Hosts
    # without a usable browser still disable them (with everything else
    # chromium-driven) via --no-wasm-e2e below.
    if args.no_wasm_e2e:
        for s in steps:
            # All chromium-driven steps (browser resource = wasm browser suite +
            # native-vs-WASM equiv sweeps + networked browser e2e), PLUS their
            # provisioning: the npm deps (wasm.npm-install) AND the wasm bundle
            # build (wasm.bundle). wasm.bundle's ONLY consumers are those browser
            # steps, so with them disabled it's orphaned — and dropping it also
            # spares a locked-down host the wasm-pack/wasm32 build it may not have.
            if ("browser" in s.resources) or s.tag in ("wasm.npm-install", "wasm.bundle"):
                disabled[s.tag] = "--no-wasm-e2e"
    if disabled:
        steps = [s for s in steps if s.tag not in disabled]

    # subset selection (carry deps along)
    by_tag = {s.tag: s for s in steps}

    def with_deps(sel):
        out, stack = set(), list(sel)
        while stack:
            t = stack.pop()
            if t in out or t not in by_tag:
                continue
            out.add(t)
            stack.extend(by_tag[t].deps)
        return out

    selected = None
    if args.only:
        selected = with_deps([x.strip() for x in args.only.split(",")])
    elif args.group:
        groups = {x.strip() for x in args.group.split(",")}
        selected = with_deps([t for t, s in by_tag.items() if s.group in groups])
    elif args.job:
        jobs = {x.strip() for x in args.job.split(",")}
        selected = with_deps([t for t, s in by_tag.items() if s.job in jobs])
    if selected is not None:
        steps = [s for s in steps if s.tag in selected]

    if args.use_prebuilt:
        # Build-once: a gating CI job produced the shared artifacts and this
        # shard downloaded them. DROP the compile producers and STRIP them from
        # every remaining step's deps (with_deps auto-pulled them in, which is
        # what otherwise forces a cold rebuild into each shard).
        PREBUILT = {"build.mtg-release", "wasm.bundle"}
        steps = [s for s in steps if s.tag not in PREBUILT]
        for s in steps:
            s.deps = [d for d in s.deps if d not in PREBUILT]
        # HARD-FAIL on a missing artifact — never fall through to a silent cold
        # cargo build inside a shard (project rule: a broken prereq handoff is
        # fatal, never papered over). Only check what THIS shard actually uses.
        if not args.list:
            tags = {s.tag for s in steps}
            missing = []
            needs_bin = any(t in tags for t in (
                "unit.nextest", "determ.commander", "determ.snapshot-resume",
                "agentplay.pytest", "agentplay.mock-game", "agentplay.mode-equiv")) \
                or any(t.startswith(("wasm.equiv", "network.")) for t in tags)
            needs_wasm = any(t in ("wasm.browser",) or t.startswith("wasm.equiv")
                             or (t.startswith("network.") and t not in (
                                 "network.equiv-random", "network.equiv-zero",
                                 "network.equiv-heuristic", "network.robots42", "network.fuzz"))
                             for t in tags)
            if needs_bin and not (os.access(PROJECT_DIR / "target/release/mtg", os.X_OK)):
                missing.append("target/release/mtg (mtg-bin artifact / MTG_REUSE_PREBUILT)")
            if needs_wasm and not (PROJECT_DIR / "web/pkg").is_dir():
                missing.append("web/pkg (wasm-pkg artifact)")
            if "unit.nextest" in tags and not os.environ.get("NEXTEST_ARCHIVE"):
                missing.append("NEXTEST_ARCHIVE env (nextest-archive artifact)")
            if missing:
                sys.stderr.write("[validate] --use-prebuilt: required prebuilt artifact(s) "
                                 "MISSING — refusing to silently cold-rebuild:\n")
                for m in missing:
                    sys.stderr.write(f"    - {m}\n")
                return 1

    if args.dot:
        _emit_dot(steps)
        return 0

    if args.list:
        for s in steps:
            dep = f"  <- {', '.join(s.deps)}" if s.deps else ""
            res = f"  [{','.join(s.resources)}]" if s.resources else ""
            print(f"{s.tag:28s} {s.desc}{res}{dep}")
        return 0

    verbosity = 0 if args.quiet else (1 + args.v)
    sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=PROJECT_DIR,
                         capture_output=True, text=True).stdout.strip() or "nosha"
    steps_dir = PROJECT_DIR / "validate_logs" / f"steps_{sha}"
    # Per-step cgroups: only when inside the delegated outer scope AND not opted
    # out. StepCgroups self-reports enabled=False off a usable cgroup setup, so
    # this is safe to construct unconditionally; the Runner then falls back to
    # killpg. (Subset/CI runs are not in-scope, so this is a no-op there.)
    cgroups = None
    if validate_cgroup is not None and not getattr(args, "no_step_cgroups", False):
        try:
            cg = validate_cgroup.StepCgroups()
            cgroups = cg if cg.enabled else None
        except Exception:
            cgroups = None
    runner = Runner(steps, args.jobs, verbosity, steps_dir,
                    resource_caps={"browser": args.browser_capacity, "net": args.net_capacity},
                    disabled=disabled, keep_going=args.keep_going, cgroups=cgroups)
    cg_note = " per-step-cgroups" if cgroups else ""
    print(f"=== validate.py: {len(steps)} steps, -j{args.jobs}, "
          f"browser-capacity={args.browser_capacity}{cg_note}, detail -> {steps_dir} ===")
    if disabled:
        by_reason = {}
        for tag, reason in sorted(disabled.items()):
            by_reason.setdefault(reason, []).append(tag)
        for reason, tags in sorted(by_reason.items()):
            print(f"=== DISABLED via {reason}: {len(tags)} step(s) — {', '.join(tags)} "
                  f"(explicit opt-out; NOT full coverage) ===")
    ok = runner.run()
    runner.print_stats()
    # NORMAL-exit backstop: cgroup.kill any step cgroup that still has live procs
    # (a setsid orphan a step left behind lives there — --collect won't reap the
    # scope while it's alive). Does NOT stop the scope, so a green run stays
    # green (exit code preserved). Signal-abort uses stop_scope instead.
    if cgroups is not None:
        leftover = cgroups.kill_all_remaining()
        if leftover:
            print(f"[validate] reaped {leftover} leftover step cgroup(s) on exit "
                  f"(setsid orphans a step left behind).")
    return 0 if ok else 1


# ===========================================================================
# Outer harness (folded in from the former scripts/validate.sh, mtg-717
# follow-on): commit-hash cache (exact + docs-only smart hit), .validate.lock,
# dirty-tree -> temporary WIP commit, clean-environment gate, CPU-utilization
# report, and the atomic validate_logs/validate_<sha>.log artifact + latest
# symlink. Runs ONLY for a full local `make validate`; subset/CI runs go
# straight through run_orchestrator() (see main()).
# ===========================================================================
SCRIPTS_DIR = PROJECT_DIR / "scripts"
LOG_DIR = PROJECT_DIR / "validate_logs"
LATEST_SYMLINK = LOG_DIR / "validate_latest.log"
LOCK_FILE = PROJECT_DIR / ".validate.lock"


class _Tee:
    """Write to several streams at once (mirrors validate.sh's `| tee`)."""
    def __init__(self, *streams):
        self.streams = streams
        self._lock = threading.Lock()

    def write(self, s):
        with self._lock:
            for st in self.streams:
                try:
                    st.write(s)
                except Exception:
                    pass

    def flush(self):
        for st in self.streams:
            try:
                st.flush()
            except Exception:
                pass


def _git(*args, check=False):
    return subprocess.run(["git", *args], cwd=PROJECT_DIR,
                          capture_output=True, text=True, check=check)


def _submodule_dirty():
    """True if any ACTIVE submodule is dirty. Excludes `update = none` (inactive)
    submodules, which legitimately show a '-' prefix (mirrors validate.sh)."""
    cfg = _git("config", "-f", ".gitmodules", "--get-regexp", r"\.update$").stdout
    inactive_paths = set()
    for line in cfg.splitlines():
        if line.endswith(" none"):
            key = line.split()[0]  # submodule.<name>.update
            name = key[len("submodule."):-len(".update")]
            p = _git("config", "-f", ".gitmodules", "--get", f"submodule.{name}.path").stdout.strip()
            if p:
                inactive_paths.add(p)
    for line in _git("submodule", "status").stdout.splitlines():
        if line[:1] in "+-U":
            parts = line.split()
            if len(parts) >= 2 and parts[1] not in inactive_paths:
                return True
    return False


def _acquire_lock():
    if LOCK_FILE.exists():
        try:
            pid = int(LOCK_FILE.read_text().strip())
        except (ValueError, OSError):
            pid = None
        alive = False
        if pid:
            try:
                os.kill(pid, 0)
                alive = True
            except (OSError, ProcessLookupError):
                alive = False
        if alive:
            sys.stderr.write(
                f"\n[validate] Another validation is already running (lock {LOCK_FILE}, PID {pid}).\n"
                f"  If stale: rm {LOCK_FILE}  (or: python3 scripts/kill_zombie_processes.py)\n")
            return False
        LOCK_FILE.unlink(missing_ok=True)  # stale
    LOCK_FILE.write_text(str(os.getpid()))
    return True


def _release_lock():
    try:
        if LOCK_FILE.exists() and LOCK_FILE.read_text().strip() == str(os.getpid()):
            LOCK_FILE.unlink(missing_ok=True)
    except OSError:
        pass


def _start_utilization():
    """Run the prehook (executed, not sourced) — it backgrounds a DISOWNED,
    long-lived sampler subshell — and parse its PID + stats file.

    CRITICAL: do NOT use capture_output/PIPE here. The sampler subshell is an
    infinite loop that INHERITS the prehook's stdout/stderr; if those are PIPEs,
    the pipe never EOFs (the sampler holds the write-end forever) and
    subprocess.run() HANGS for the whole validate run. We capture the prehook's
    short banner to a TEMP FILE instead (a file fd can't block the parent) and
    send stderr to DEVNULL. (The prehook ALSO redirects the sampler's own fds to
    /dev/null as defense-in-depth.) The prehook parent exits promptly; the
    disowned sampler keeps running with only harmless file/null fds."""
    pre = SCRIPTS_DIR / "utilization_prehook.sh"
    if not pre.exists():
        return None
    import tempfile
    fd, banner_path = tempfile.mkstemp(prefix="util_prehook_", suffix=".out")
    try:
        with os.fdopen(fd, "w") as outf:
            # stdout -> file (not PIPE), stderr -> DEVNULL: subprocess.run waits
            # only for the prehook PARENT to exit, never for a pipe the sampler holds.
            subprocess.run(["bash", str(pre)], cwd=PROJECT_DIR,
                           stdout=outf, stderr=subprocess.DEVNULL)
        banner = open(banner_path).read()
    finally:
        try:
            os.unlink(banner_path)
        except OSError:
            pass
    sys.stdout.write(banner)
    pid = stats = None
    for line in banner.splitlines():
        if "PID:" in line:
            pid = line.split("PID:")[1].strip().rstrip(")").strip()
        elif line.startswith("Stats file:"):
            stats = line.split(":", 1)[1].strip()
    if pid and stats:
        return {"pid": pid, "stats": stats, "start": str(int(time.time()))}
    return None


def _stop_utilization(mon):
    """Run the posthook with the prehook's vars in the env; tee its report."""
    if not mon:
        return
    post = SCRIPTS_DIR / "utilization_posthook.sh"
    if not post.exists():
        return
    env = dict(os.environ, UTIL_MONITOR_PID=mon["pid"], UTIL_STATS_FILE=mon["stats"],
               UTIL_START_TIME=mon["start"])
    r = subprocess.run(["bash", str(post)], cwd=PROJECT_DIR, env=env,
                       capture_output=True, text=True)
    sys.stdout.write(r.stdout)
    if r.stderr:
        sys.stdout.write(r.stderr)


_SCOPE_PROBE = None


def _systemd_scope_available():
    """True iff `systemd-run --user --scope` actually works here (cached). The
    user verified this on the dev box (systemd 255, cgroup v2, user-delegated)."""
    global _SCOPE_PROBE
    if _SCOPE_PROBE is None:
        if not shutil.which("systemd-run"):
            _SCOPE_PROBE = False
        else:
            try:
                r = subprocess.run(
                    ["systemd-run", "--user", "--scope", "--quiet",
                     f"--unit=validate-probe-{os.getpid()}", "true"],
                    capture_output=True, timeout=8)
                _SCOPE_PROBE = (r.returncode == 0)
            except (subprocess.TimeoutExpired, OSError):
                _SCOPE_PROBE = False
    return _SCOPE_PROBE


def _resolve_mem_budget(args):
    """Resolve the outer-scope MemoryMax (bytes). A cap is applied BY DEFAULT —
    a runaway (e.g. the Return-the-Favor infinite self-copy loop that ballooned
    one `mtg` to ~40 GB and wedged the box, 2026-06-09) must NEVER be able to
    OOM the host; the cgroup OOM-kills it at the cap instead. Resolution order:

      * explicit `--max-mem SPEC`  -> that (absolute size, or 'auto' = 80% of
        current MemAvailable);
      * `--no-max-mem`             -> None (uncapped — DANGEROUS, opt-out only);
      * DEFAULT (no flag)          -> VALIDATE_TOTAL_RSS_BASELINE_BYTES * 1.25
        (the characterized typical whole-run peak RSS x the 1.25 safety factor),
        but never less than a floor and never more than 85% of total RAM (so a
        small-RAM box still gets a workable cap and a big-RAM box still leaves
        headroom for the OS + concurrent cross-slot validates).

    The baseline is defined in ONE place (VALIDATE_TOTAL_RSS_BASELINE_BYTES,
    below) so the actionable-OOM message can point an agent at exactly where to
    confirm-and-bump it."""
    if getattr(args, "no_max_mem", False):
        return None
    spec = getattr(args, "max_mem", None)
    if spec:
        if str(spec).lower() == "auto":
            avail = mem_available_bytes()
            return int(avail * 0.8) if avail else None
        return parse_size(spec)
    # DEFAULT cap: 1.25x the characterized whole-run peak, clamped to [floor, 85% RAM].
    default_cap = int(VALIDATE_TOTAL_RSS_BASELINE_BYTES * MEM_CAP_FACTOR)
    total = _total_ram_bytes()
    if total:
        default_cap = min(default_cap, int(total * 0.85))
    return max(default_cap, MEM_CAP_FLOOR_BYTES)


def _maybe_reexec_in_scope(args):
    """mtg-743: re-exec a FULL local validate inside a transient systemd
    --user SCOPE (a cgroup), so EVERY descendant — incl. setsid/double-forked
    escapees the per-step killpg reaper can't catch (orphan mtg server /
    http.server / chromium / util sampler) — is contained and reaped atomically
    when the run ends or via `systemctl --user stop validate-*.scope` (see
    kill_zombie_processes.py). This is what makes concurrent cross-slot validates
    safe by default + kills the stale-lock / port-collision false-positive class.

    SUPPRESSED (runs directly, never breaks validate) when: already inside the
    scope (anti-recursion sentinel) / in CI / --no-scope / systemd-run
    unavailable. Re-exec uses the RELATIVE script path (sys.argv[0]) so the
    scoped process's cmdline does NOT contain the worktree path — otherwise
    check_clean_environment.py would flag the scoped validate as a conflicting
    process (the mtg-463 self-detection footgun)."""
    if os.environ.get("MTG_VALIDATE_IN_SCOPE") == "1":
        return  # already re-exec'd into the scope (anti-recursion)
    if args.no_scope or os.environ.get("CI") or os.environ.get("GITHUB_ACTIONS"):
        return
    if not _systemd_scope_available():
        print("[validate] systemd --user scope unavailable — running unscoped "
              "(per-step reaper only; pass --no-scope to silence).")
        return
    unit = f"validate-{os.getpid()}"
    # Delegate=yes makes the scope a DELEGATED cgroup so the in-scope runner can
    # carve per-step CHILD cgroups under it (validate_cgroup.StepCgroups) for
    # setsid-proof per-step teardown via cgroup.kill. The outer-scope stop still
    # flushes the whole subtree regardless. (Verified on systemd 255 / cgroup v2.)
    props = ["-p", "Delegate=yes"]
    mem_max = _resolve_mem_budget(args)
    if mem_max:
        # MemoryMax = hard cap (cgroup OOM-kills the run if exceeded — contains a
        # runaway validate); MemoryHigh = soft throttle at 90% (reclaim pressure
        # before the hard kill). Together they keep concurrent cross-slot
        # validates from OOM-thrashing the host. APPLIED BY DEFAULT (mtg-887).
        # MemorySwapMax=0: OOM-KILL a runaway at the cap, do NOT let it swap (a
        # 40 GB runaway swapping into 18 GB still thrashes the host — the exact
        # wedge this guards against).
        props += ["-p", f"MemoryMax={mem_max}", "-p", f"MemoryHigh={int(mem_max * 0.9)}",
                  "-p", "MemorySwapMax=0"]
        src = "explicit --max-mem" if getattr(args, "max_mem", None) else \
              "DEFAULT (1.25x VALIDATE_TOTAL_RSS_BASELINE_BYTES in scripts/validate.py)"
        print(f"[validate] outer scope memory cap: MemoryMax={_fmt_bytes(mem_max)} "
              f"(MemoryHigh={_fmt_bytes(int(mem_max * 0.9))}, swap=0) — {src}.")
    elif getattr(args, "no_max_mem", False):
        print("[validate] ⚠ --no-max-mem: outer scope is UNCAPPED — a runaway test can OOM the "
              "HOST (the 2026-06-09 box-wedge). Use only for deliberate memory profiling.")
    cmd = ["systemd-run", "--user", "--scope", "--collect", "--quiet",
           f"--unit={unit}", *props, "--setenv=MTG_VALIDATE_IN_SCOPE=1",
           f"--setenv=MTG_VALIDATE_SCOPE_UNIT={unit}.scope",
           "--", sys.executable, sys.argv[0], *sys.argv[1:]]
    print(f"[validate] re-exec inside transient systemd scope {unit}.scope "
          f"(two-level cgroup; full-descendant cleanup on exit)…")
    sys.stdout.flush()
    try:
        os.execvp("systemd-run", cmd)  # replaces this process
    except OSError as e:
        print(f"[validate] systemd-run exec failed ({e}) — continuing unscoped.")


def _install_scope_teardown():
    """Inside the scope, make Ctrl-C / `kill` of the runner tear down the WHOLE
    cgroup — not just this PID. THE GAP THIS CLOSES: killing only the scoped
    runner leaves setsid-escapee orphans (mtg server / chromium) alive in the
    scope cgroup (verified empirically — killpg AND `--collect` both miss them).
    The fix: on SIGINT/SIGTERM, `systemctl --user stop` our OWN scope — that
    SIGKILLs every child step cgroup + every escapee atomically (the stop
    cascades because the step cgroups are genuinely nested under the scope), and
    kills us too (intended: an aborted run exits with the signal code).

    The NORMAL-exit backstop is handled separately in run_orchestrator via
    StepCgroups.kill_all_remaining() — which does NOT stop the scope, so a
    SUCCESSFUL run's exit code is preserved (stopping our own scope from atexit
    would SIGTERM us and turn a green run red).

    No-op when not in-scope (subset/CI/--no-scope) or systemctl is absent."""
    unit = os.environ.get("MTG_VALIDATE_SCOPE_UNIT")
    if not unit or os.environ.get("MTG_VALIDATE_IN_SCOPE") != "1":
        return
    if validate_cgroup is None or not shutil.which("systemctl"):
        return
    # Resolve the scope cgroup path NOW (from /proc/self/cgroup), not inside the
    # handler — so the handler's cgroup.kill is a single fast file-write with no
    # systemctl shell-out (which can stall under load and let the fallback
    # `systemctl stop` wait ~10s on chromium's ignored SIGTERM).
    scope_cg = validate_cgroup.scope_cgroup_from_self()

    def _on_signal(signum, _frame):
        # Stopping our own scope kills us too — that is intended (atomic, SIGKILL-
        # proof teardown of the whole tree). Print first so the reason is visible.
        try:
            sys.stderr.write(f"\n[validate] signal {signum} — stopping scope {unit} "
                             f"(tears down all steps + orphans)…\n")
            sys.stderr.flush()
        except Exception:
            pass
        # Release our lock NOW: the run_with_harness `finally` that normally does
        # this won't run once stop_scope SIGKILLs us. (The next run's stale-PID
        # check would also reap it, but removing it here avoids the confusing
        # "lock remains" window.)
        _release_lock()
        validate_cgroup.stop_scope(unit, scope_cg)
        # If the stop somehow didn't kill us, exit non-zero with the signal code.
        os._exit(128 + signum)

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            signal.signal(sig, _on_signal)
        except (ValueError, OSError):
            pass


def run_with_harness(args):
    # 0. self-isolate: re-exec inside a transient systemd --user scope so the
    #    whole descendant tree is reaped atomically on exit (mtg-743). No-op
    #    when already scoped / CI / --no-scope / systemd unavailable.
    _maybe_reexec_in_scope(args)
    # 0b. Inside the scope now: install signal + exit teardown that stops the
    #     whole scope cgroup (catches setsid escapees killpg/--collect miss).
    _install_scope_teardown()
    # 1. clean-environment gate
    envcheck = SCRIPTS_DIR / "check_clean_environment.py"
    if envcheck.exists():
        if subprocess.run(["python3", str(envcheck)], cwd=PROJECT_DIR).returncode != 0:
            sys.stderr.write("\n[validate] Environment not clean — conflicting processes detected.\n"
                             "  Clean up with: python3 scripts/kill_zombie_processes.py\n")
            return 1

    # 2. lock
    if not _acquire_lock():
        return 1

    created_wip = False
    disable_cache = False
    try:
        # 3. dirty-tree -> WIP commit (or run dirty with caching off)
        _git("update-index", "--refresh", "-q")
        has_regular = _git("diff-index", "--quiet", "HEAD", "--").returncode != 0
        has_submod = _submodule_dirty()
        if has_regular or has_submod:
            print("\n[validate] Working copy dirty — running `cargo fmt --all` first…")
            subprocess.run(["cargo", "fmt", "--all"], cwd=PROJECT_DIR)
            if args.no_wip_commit:
                print("[validate] --no-wip-commit: running WITHOUT a WIP commit; caching disabled.")
                disable_cache = True
            elif has_submod and not args.force_wip_commit:
                sys.stderr.write(
                    "\n[validate] Submodule changes detected — refusing to bury them in a WIP commit.\n"
                    "  Commit/stash the submodule change, or pass --force-wip-commit / --no-wip-commit.\n")
                return 1
            else:
                _git("add", "-A")
                _git("commit", "-m", "wip", "--no-verify")
                created_wip = True
                print("[validate] Created temporary WIP commit (auto-reverted on exit).")

        # 4. resolve log path
        sha = _git("rev-parse", "HEAD").stdout.strip() or "nosha"
        LOG_DIR.mkdir(parents=True, exist_ok=True)
        suffix = "_dirty" if created_wip else ""
        log_file = LOG_DIR / f"validate_{sha}{suffix}.log"
        if args.force and log_file.exists():
            n = 2
            while (LOG_DIR / f"validate_{sha}{suffix}_{n}.log").exists():
                n += 1
            log_file = LOG_DIR / f"validate_{sha}{suffix}_{n}.log"

        # 5. cache (skipped under --force / dirty-no-wip)
        if not args.force and not disable_cache:
            if log_file.exists():
                print(f"\n[validate] ✓ cache hit for {sha} — already passed ({log_file}).")
                return 0
            hit = _smart_cache_hit(sha, log_file)
            if hit:
                return 0

        # 6. run, tee stdout/stderr to a .wip log
        wip = log_file.with_suffix(".log.wip")
        print("=" * 60)
        print(f"[validate] running — commit {sha}{' (dirty)' if created_wip else ''} -> {log_file}")
        print("=" * 60)
        mon = None if args.no_monitor_utilization else _start_utilization()
        rc = 1
        old_out, old_err = sys.stdout, sys.stderr
        with open(wip, "w") as logf:
            tee = _Tee(old_out, logf)
            sys.stdout = sys.stderr = tee
            try:
                rc = run_orchestrator(args)
                # Footprint: peak RSS of the WHOLE validate scope cgroup (every
                # step), read precisely from cgroup-v2 memory.peak — no sampling.
                # In-scope only; printed into the tee'd log artifact.
                if validate_cgroup is not None:
                    peak = validate_cgroup.scope_memory_peak()
                    if peak:
                        print(f"[validate] peak memory (whole-run scope): "
                              f"{_fmt_bytes(peak)}")
            finally:
                _stop_utilization(mon)
                sys.stdout, sys.stderr = old_out, old_err

        # 7. artifact (only cache successes)
        if rc == 0:
            os.replace(wip, log_file)
            LATEST_SYMLINK.unlink(missing_ok=True)
            try:
                LATEST_SYMLINK.symlink_to(log_file.name)
            except OSError:
                pass
            print(f"\n[validate] ✓ PASS — cached to {log_file}")
        else:
            try:
                os.replace(wip, log_file.with_suffix(".log.failed"))
            except OSError:
                pass
            print("\n[validate] ✗ FAIL — see the dumped step detail above.")
        return rc
    finally:
        if created_wip:
            _git("reset", "--soft", "HEAD~1")
        _release_lock()


def _smart_cache_hit(sha, log_file):
    """Docs-only smart hit: if the only diff from the last validated commit is
    *.md, reuse its log (symlink) instead of re-running. Mirrors validate.sh."""
    if not LATEST_SYMLINK.is_symlink():
        return False
    target = os.readlink(LATEST_SYMLINK)
    m = re.match(r"validate_([0-9a-f]+)(_dirty)?\.log", os.path.basename(target))
    if not m:
        return False
    prev = m.group(1)
    if len(prev) != 40 or prev == sha:
        return False
    if _git("cat-file", "-e", prev).returncode != 0 or _git("cat-file", "-e", sha).returncode != 0:
        return False
    diff = _git("diff", "--name-only", prev, sha)
    if diff.returncode != 0:
        return False
    changed = [f for f in diff.stdout.splitlines() if f.strip()]
    if changed and all(f.endswith(".md") for f in changed):
        try:
            log_file.symlink_to(os.path.basename(target))
            LATEST_SYMLINK.unlink(missing_ok=True)
            LATEST_SYMLINK.symlink_to(log_file.name)
        except OSError:
            return False
        print(f"\n[validate] ✓ smart cache hit (docs-only changes since {prev[:12]}): "
              f"{_fmt_files(changed)}")
        return True
    if changed:
        # Code changed -> cache miss, will re-run. Show WHY, but CAP the list so a
        # big diff doesn't flood the log (user request).
        print(f"\n[validate] code changed since {prev[:12]} — re-validating: {_fmt_files(changed)}")
    return False


def _fmt_files(files, cap=20):
    """Join a file list, capped: first `cap` then '... and N more (M total)'."""
    if len(files) <= cap:
        return ", ".join(files)
    return ", ".join(files[:cap]) + f", … and {len(files) - cap} more ({len(files)} total)"


if __name__ == "__main__":
    sys.exit(main())
