#!/usr/bin/env python3
"""validate_run.py — the `make validate` orchestrator (mtg-717).

This is the SINGLE SOURCE OF TRUTH for what `make validate` runs and how. It
replaces the Makefile's `validate-*-step` / `validate-parallel-steps` /
`validate-impl` apparatus and `scripts/validate_step.sh`. The Makefile keeps
only thin build-primitive targets (build, wasm-dev, clippy, …) that this runner
invokes; CI shards call THIS with `--group <X>` so CI can never drift from local.

It owns:
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
  validate_run.py [--jobs N] [--group G[,G2]] [--only G.J[,…]] [--job J[,…]]
                  [-q|-v|-vv] [--list] [--no-network] [--browser-capacity N]

Exit status: 0 if all selected steps pass, 1 otherwise.
"""

import argparse
import os
import re
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
    add(Step("wasm", "npm-install", "web/ npm install (e2e deps)",
             f"cd web && {NPM} install --silent 2>/dev/null"))
    add(Step("wasm", "browser", "WASM browser e2e suite (12 playwright tests)",
             "cd web && " + " && ".join(
                 f"{NODE} {t}" for t in [
                     "test_fancy_tui.js", "test_human_input.js", "test_click_and_log.js",
                     "test_font_size_layout.js", "test_decouple_step3_launch_game_session.js",
                     "test_card_size_stability.js", "test_battlefield_layout.js",
                     "test_decouple_step6_valid_choices.js", "test_tapped_rotation.js",
                     "test_graveyard_overlay.js", "test_deck_editor.js",
                     "test_cdn_image_table.js"]),
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
        ("playwright-check", "npm install + verify chromium provisioned",
         f"cd web && {NPM} install --silent 2>/dev/null || true; {NODE} playwright_check.js"),
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
    def __init__(self, steps, jobs, verbosity, steps_dir, resource_caps):
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
                if len(self.done) + len(self._skipped()) >= len(self.steps) and not self.running:
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
        proc = subprocess.Popen(["bash", "-c", step.cmd], cwd=PROJECT_DIR, env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

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
            # Genuine hang: SIGKILL the direct child (the bash leader). We do
            # NOT killpg the group — in some sandboxes killpg can resolve to and
            # kill the runner's own group. Killing the leader is enough to
            # unblock; orphan grandchildren (if any) are harmless here.
            timed_out = True
            try:
                proc.kill()
                proc.wait(timeout=10)
            except Exception:
                pass
        # The daemon reader may still be blocked on an orphan-held pipe (a test
        # left a server/chromium holding stdout). That is exactly the old hang —
        # but because the reader is a DAEMON and we don't block on it, the step
        # completes regardless. proc.wait() already returned once the test
        # process itself exited; orphans don't gate us. Brief join, then move on.
        reader.join(timeout=2)
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
            self.done[step.tag] = (ok, dur, summary)
            self._release(step)
            if not ok:
                self.failed = True
                self.stop = True
        if ok:
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
        npass = sum(1 for _, ok, _, _ in results if ok)
        nfail = sum(1 for _, ok, _, _ in results if not ok)
        print(f"  result: {npass} passed, {nfail} failed, {len(skipped)} skipped")
        print("=" * 60)


def main():
    ap = argparse.ArgumentParser(description="make validate orchestrator (mtg-717)")
    ap.add_argument("--jobs", "-j", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--group", help="comma-separated jobGroups to run (CI shard)")
    ap.add_argument("--only", help="comma-separated group.job to run (+their deps)")
    ap.add_argument("--job", help="comma-separated jobIds to run (any group)")
    ap.add_argument("--no-network", action="store_true", help="skip the network jobGroup")
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
    args = ap.parse_args()

    steps = build_registry()
    if args.no_network:
        steps = [s for s in steps if not s.networkonly]

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
                sys.stderr.write("[validate_run] --use-prebuilt: required prebuilt artifact(s) "
                                 "MISSING — refusing to silently cold-rebuild:\n")
                for m in missing:
                    sys.stderr.write(f"    - {m}\n")
                return 1

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
                    resource_caps={"browser": args.browser_capacity, "net": args.net_capacity})
    print(f"=== validate_run.py: {len(steps)} steps, -j{args.jobs}, "
          f"browser-capacity={args.browser_capacity}, detail -> {steps_dir} ===")
    ok = runner.run()
    runner.print_stats()
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
