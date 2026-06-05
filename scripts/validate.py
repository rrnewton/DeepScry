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

# mtg-zsi9f / mtg-o99ow native-first pivot: the WASM browser e2e steps that
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
# chromium / starve the timing-sensitive networked games. Default capacity 1
# reproduces the proven safe one-browser-at-a-time behaviour while CPU steps
# (build/clippy/nextest/examples) run fully concurrently. Raise with
# --browser-capacity N to characterize e2e parallel speedup.


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
    # build-once) has no binary unless we declare it. (mtg-uxslu)
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
    add(Step("wasm", "browser", "WASM browser e2e suite (16 playwright tests)",
             "cd web && " + " && ".join(
                 f"{NODE} {t}" for t in [
                     "test_fancy_tui.js", "test_bug_report.js", "test_human_input.js",
                     "test_click_and_log.js", "test_font_size_layout.js",
                     "test_decouple_step3_launch_game_session.js",
                     "test_card_size_stability.js", "test_battlefield_layout.js",
                     "test_decouple_step6_valid_choices.js", "test_tapped_rotation.js",
                     "test_graveyard_overlay.js", "test_deck_editor.js",
                     "test_cdn_image_table.js", "test_image_flicker_memo.js",
                     "test_aura_render.js", "test_render_skip.js"]),
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
                 keep_going=False):
        self.disabled = dict(disabled or {})  # tag -> flag reason (explicit opt-out)
        # EAGER-EXIT (default): on the FIRST step failure, kill the steps still
        # running in parallel (via their process groups) and stop immediately,
        # instead of letting the in-flight wave finish. --keep-going flips this
        # to "run everything, report all failures in one pass".
        self.keep_going = keep_going
        self.running_pgids = {}  # tag -> pgid of an in-flight step (for eager kill)
        self.aborted = set()     # tags killed by eager-exit (labelled, not FAIL)
        self.steps = {s.tag: s for s in steps}
        self.order = [s.tag for s in steps]
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

    def _reap(self, pgid):
        """SIGKILL the step's entire process group (orphan grandchildren incl.).
        Takes the pgid captured right after Popen — NOT os.getpgid(proc.pid),
        which fails once proc.wait() has reaped the leader (the group still
        exists while any grandchild lives, so the stored pgid stays valid).
        Scoped to the step's own session via start_new_session; guarded so we
        never signal the runner's own group (suicide / the historical
        exit-144)."""
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
        # WITHOUT ever touching the runner's own group. mtg-ibj22: a failed/hung
        # browser test used to leak chromium/server orphans that held resources
        # (and the stdout pipe) into later steps; scoped-killpg reaps them. The
        # historical exit-144 came from killpg WITHOUT start_new_session (the
        # child shared the runner's pgid, so killpg was suicide) — the new
        # session makes it safe; _reap() additionally guards against our pgrp.
        proc = subprocess.Popen(["bash", "-c", step.cmd], cwd=PROJECT_DIR, env=env,
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
            self._reap(step_pgid)
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
        # mtg-ibj22: reap the step's whole process group, so any orphan
        # grandchildren (mtg server / http.server / chromium that outlived the
        # test) are SIGKILLed now instead of leaking into later steps or holding
        # the stdout pipe. Scoped to the step's own session (start_new_session
        # above) — never the runner's group. This also lets the abandoned reader
        # thread finally see EOF.
        self._reap(step_pgid)
        try:
            fh.close()
        except Exception:
            pass
        dur = round(time.time() - start)
        ok = (proc.returncode == 0) and not timed_out
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
                        self._reap(pgid)           # SIGKILL its process group -> its proc.wait returns
        if was_aborted:
            self._emit(f"[{step.tag}] ⊘ ABORT  {step.desc} ({dur}s — eager-exit after another step failed; --keep-going to run all)")
        elif ok:
            extra = f"  [{summary}]" if (summary and self.verbosity >= 1) else ""
            self._emit(f"[{step.tag}] ✓ PASS   {step.desc} ({dur}s){extra}")
        else:
            why = f"TIMEOUT >{step.timeout}s" if timed_out else f"exit {proc.returncode}"
            self._emit(f"[{step.tag}] ✗ FAIL   {step.desc} ({dur}s, {why})")
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


def main():
    ap = argparse.ArgumentParser(description="make validate orchestrator (mtg-717)")
    ap.add_argument("--jobs", "-j", type=int, default=os.cpu_count() or 4)
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
                         "(mtg-o99ow: the WASM reorder/reveal split + apply-frontier fix landed). "
                         "Flag retained so existing invocations do not error; use --no-wasm-e2e to "
                         "disable all browser/chromium steps on a host without a usable browser.")
    ap.add_argument("--browser-capacity", type=int, default=1,
                    help="how many chromium-heavy steps may run at once (default 1)")
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
    args = ap.parse_args()

    if args.sequential:
        args.jobs = 1
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
    # mtg-zsi9f / mtg-o99ow: the WASM network-GAME e2e steps were temporarily
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
    runner = Runner(steps, args.jobs, verbosity, steps_dir,
                    resource_caps={"browser": args.browser_capacity, "net": args.net_capacity},
                    disabled=disabled, keep_going=args.keep_going)
    print(f"=== validate.py: {len(steps)} steps, -j{args.jobs}, "
          f"browser-capacity={args.browser_capacity}, detail -> {steps_dir} ===")
    if disabled:
        by_reason = {}
        for tag, reason in sorted(disabled.items()):
            by_reason.setdefault(reason, []).append(tag)
        for reason, tags in sorted(by_reason.items()):
            print(f"=== DISABLED via {reason}: {len(tags)} step(s) — {', '.join(tags)} "
                  f"(explicit opt-out; NOT full coverage) ===")
    ok = runner.run()
    runner.print_stats()
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


def _maybe_reexec_in_scope(args):
    """mtg-ibj22: re-exec a FULL local validate inside a transient systemd
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
    cmd = ["systemd-run", "--user", "--scope", "--collect", "--quiet",
           f"--unit={unit}", "--setenv=MTG_VALIDATE_IN_SCOPE=1",
           "--", sys.executable, sys.argv[0], *sys.argv[1:]]
    print(f"[validate] re-exec inside transient systemd scope {unit}.scope "
          f"(full-descendant cleanup on exit)…")
    sys.stdout.flush()
    try:
        os.execvp("systemd-run", cmd)  # replaces this process
    except OSError as e:
        print(f"[validate] systemd-run exec failed ({e}) — continuing unscoped.")


def run_with_harness(args):
    # 0. self-isolate: re-exec inside a transient systemd --user scope so the
    #    whole descendant tree is reaped atomically on exit (mtg-ibj22). No-op
    #    when already scoped / CI / --no-scope / systemd unavailable.
    _maybe_reexec_in_scope(args)
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
