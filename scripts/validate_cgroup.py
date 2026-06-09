#!/usr/bin/env python3
"""validate_cgroup.py — two-level cgroup teardown for validate.py (mtg-726/mtg-862).

WHY THIS EXISTS
---------------
`make validate` re-execs itself inside a transient `systemd-run --user --scope`
(an OUTER cgroup) so the whole descendant tree is contained. But three
empirically-verified gaps remained (probed on this dev box, systemd 255, cgroup
v2, user-delegated cpu/memory/pids — see the commit body for the probe results):

  1. Per-step teardown used `killpg`, which a `setsid`/double-forked grandchild
     (orphan `mtg server` / `http.server` / chromium) ESCAPES — the exact
     zombie/port-collision/false-desync class this workstream keeps hitting.
  2. On NORMAL exit, a setsid orphan left in the OUTER scope's cgroup keeps the
     scope `active running`; `--collect` does NOT garbage-collect a scope that
     still has live processes, so the orphan survives the run.
  3. Nothing STOPPED the outer scope on Ctrl-C / `kill` of the runner — only a
     separate, after-the-fact `kill_zombie_processes.py` did.

THE MODEL (two levels, both real cgroups — NOT sibling scopes)
--------------------------------------------------------------
A transient `systemd-run --user --scope` lands every unit as a SIBLING under
`app.slice` — so a naive per-step `systemd-run --scope` would NOT be torn down
by stopping the outer scope (verified: stopping the parent scope does not
cascade to a sibling child scope). Instead we make the OUTER scope a DELEGATED
cgroup (`-p Delegate=yes`) and manage genuine CHILD cgroups by hand:

    app.slice/validate-<pid>.scope/         <- outer scope (delegated)
        ├── supervisor/                      <- the runner itself lives here
        │                                       (cgroup v2 "no internal
        │                                       processes" rule: a cgroup with
        │                                       child cgroups may not also hold
        │                                       processes, so the runner must
        │                                       vacate the scope root)
        ├── step-build.mtg-release/          <- one child cgroup per step;
        ├── step-network.gui/                   the step's bash leader self-moves
        └── …                                   here as its FIRST action

Per-step teardown is then `echo 1 > step-<tag>/cgroup.kill`, which SIGKILLs the
WHOLE subtree atomically — including setsid escapees (setsid changes only
session/pgid, never cgroup membership, so a grandchild cannot leave the step's
cgroup). Whole-run teardown is `systemctl --user stop <outer>.scope`, which
flushes every child cgroup (they are genuinely nested, so the stop cascades).

GRACEFUL DEGRADATION
--------------------
Everything here is best-effort. If `Delegate=yes` was not granted, the
delegated cgroup can't be found, or the host has no cgroup v2 / systemd, the
helper reports `enabled == False` and the caller falls back to the existing
`killpg` reaper. cgroups are an ADDITIONAL, stronger reaper, never a hard
dependency — validate must still run on a locked-down box without delegation.
"""

import os
import re
import subprocess
from pathlib import Path

CGROUP_ROOT = Path("/sys/fs/cgroup")
_SUPERVISOR = "supervisor"


def _sanitize(tag: str) -> str:
    """A cgroup directory name for a step tag. cgroup v2 names may not contain
    '/'; keep it readable (group.job -> step-group.job) but strip anything odd."""
    safe = re.sub(r"[^A-Za-z0-9._-]", "_", tag)
    return f"step-{safe}"


def _my_cgroup_path() -> Path | None:
    """Filesystem path of THIS process's cgroup v2, via /proc/self/cgroup
    (`0::<path>` for the unified hierarchy). None if not cgroup v2."""
    try:
        for line in Path("/proc/self/cgroup").read_text().splitlines():
            if line.startswith("0::"):
                rel = line[3:].lstrip("/")
                return CGROUP_ROOT / rel
    except OSError:
        pass
    return None


class StepCgroups:
    """Manages per-step child cgroups under the delegated outer validate scope.

    Lifecycle:
      * construct once (in the in-scope runner). `enabled` tells the caller
        whether per-step cgroups are usable; if not, it must use killpg.
      * `prepare_command(tag, cmd)` wraps a step's shell command so the step's
        bash leader self-moves into its child cgroup BEFORE forking any
        grandchild (the cgroup-v2 fork-inheritance contract).
      * `kill(tag)` SIGKILLs the step's whole subtree (setsid-proof).
      * `cleanup(tag)` removes the now-empty child cgroup dir (best-effort).
    """

    def __init__(self):
        self.enabled = False
        self.root: Path | None = None  # the delegated scope cgroup root
        self._made: set[str] = set()
        # Only meaningful inside the scope (the re-exec sets this).
        if os.environ.get("MTG_VALIDATE_IN_SCOPE") != "1":
            return
        scope_cg = _my_cgroup_path()
        if scope_cg is None or not scope_cg.is_dir():
            return
        # Verify the unified controllers we need are available to delegate to
        # children. The outer scope was created with Delegate=yes, so its
        # cgroup.controllers should list cpu/memory/pids.
        try:
            controllers = (scope_cg / "cgroup.controllers").read_text().split()
        except OSError:
            return
        # Move the supervisor (this runner + its threads) OUT of the scope root
        # into a child cgroup, so the root holds no processes and may gain child
        # cgroups (cgroup v2 no-internal-processes rule). Then enable the
        # controllers for children via subtree_control.
        sup = scope_cg / _SUPERVISOR
        try:
            sup.mkdir(exist_ok=True)
            # Move the whole thread-group: writing the leader pid to
            # cgroup.procs migrates every thread of the process.
            (sup / "cgroup.procs").write_text(str(os.getpid()))
            want = " ".join(f"+{c}" for c in ("cpu", "memory", "pids") if c in controllers)
            if want:
                try:
                    (scope_cg / "cgroup.subtree_control").write_text(want)
                except OSError:
                    # Non-fatal: even without delegated controllers, cgroup.kill
                    # on a child still works (it's a core cgroup-v2 file). We
                    # only lose per-step cpu/mem accounting, not the kill.
                    pass
        except OSError:
            return
        self.root = scope_cg
        self.enabled = True

    def prepare_command(self, tag: str, cmd: str) -> str:
        """Return the step command wrapped so its bash leader joins the step's
        child cgroup FIRST (before forking grandchildren). No-op string-wrap
        when disabled. The self-move is best-effort (`|| true`): if it fails the
        step still runs and the outer-scope reaper remains the backstop."""
        if not self.enabled or self.root is None:
            return cmd
        child = self.root / _sanitize(tag)
        try:
            child.mkdir(exist_ok=True)
            self._made.add(tag)
        except OSError:
            return cmd
        procs = child / "cgroup.procs"
        # $$ is the bash leader's own pid. Writing it migrates the leader; every
        # subsequently-forked child/grandchild inherits this cgroup at fork.
        # Redirect errors so a delegation hiccup can't corrupt the step's stdout.
        return f'echo $$ > {procs} 2>/dev/null || true\n{cmd}'

    def kill(self, tag: str) -> bool:
        """SIGKILL the step's entire cgroup subtree (setsid escapees included).
        Returns True if the kill file was written. Best-effort / never raises."""
        if not self.enabled or self.root is None or tag not in self._made:
            return False
        killf = self.root / _sanitize(tag) / "cgroup.kill"
        try:
            killf.write_text("1")
            return True
        except OSError:
            return False

    def cleanup(self, tag: str) -> None:
        """Remove the step's (now-empty) child cgroup directory. Best-effort:
        rmdir fails with EBUSY if procs remain, which is fine — the outer-scope
        stop will flush it at the end of the run."""
        if not self.enabled or self.root is None or tag not in self._made:
            return
        try:
            (self.root / _sanitize(tag)).rmdir()
        except OSError:
            pass
        self._made.discard(tag)

    def kill_all_remaining(self) -> int:
        """NORMAL-EXIT backstop: cgroup.kill + rmdir EVERY step child cgroup we
        ever created that still exists. Catches a setsid orphan a step left
        behind (it stays in that step's cgroup; --collect won't reap a scope
        with live procs, and per-step rmdir failed with EBUSY while it lived).
        Does NOT touch the supervisor cgroup, so it never kills the runner — the
        exit code is preserved (unlike `systemctl stop <scope>`). Returns the
        count of step cgroups that still existed."""
        if not self.enabled or self.root is None:
            return 0
        n = 0
        # Scan the real directory, not just self._made: a step whose cleanup ran
        # may be gone, while a crashed step's dir lingers.
        try:
            children = [p for p in self.root.iterdir()
                        if p.is_dir() and p.name.startswith("step-")]
        except OSError:
            children = []
        for child in children:
            n += 1
            try:
                (child / "cgroup.kill").write_text("1")
            except OSError:
                pass
            try:
                child.rmdir()
            except OSError:
                pass
        return n


def _scope_cgroup_path(unit: str) -> Path | None:
    """Filesystem cgroup path of a --user transient scope (via systemctl)."""
    try:
        r = subprocess.run(
            ["systemctl", "--user", "show", unit, "--property=ControlGroup", "--value"],
            capture_output=True, text=True, timeout=8)
    except (subprocess.TimeoutExpired, OSError):
        return None
    cg = r.stdout.strip()
    return (CGROUP_ROOT / cg.lstrip("/")) if cg else None


def stop_scope(unit: str) -> bool:
    """Tear down the whole outer scope. Two-step for SPEED + cleanliness:

      1. `cgroup.kill` the scope's cgroup directly — an INSTANT, atomic SIGKILL
         of every member (incl. chromium, which IGNORES the SIGTERM that
         `systemctl stop` sends first). Without this, `systemctl stop` sits in
         `stop-sigterm` for the scope's full TimeoutStopSec waiting on chromium.
      2. `systemctl --user stop` to deactivate + GC the (now-empty) unit.

    Either step alone flushes the descendants; doing both makes teardown both
    immediate and tidy. SIGKILL-proof, setsid-proof. Best-effort throughout."""
    if not unit:
        return False
    cg = _scope_cgroup_path(unit)
    if cg is not None:
        try:
            (cg / "cgroup.kill").write_text("1")  # instant SIGKILL of whole tree
        except OSError:
            pass
    try:
        subprocess.run(["systemctl", "--user", "stop", unit],
                       capture_output=True, timeout=15)
        return True
    except (subprocess.TimeoutExpired, OSError):
        return False
