---
title: cgroup-isolate validate runs (transient systemd scope) -- kill all descendants as a unit, fix zombie mtg-server/http.server/chromium orphans
status: open
priority: 2
issue_type: task
labels:
- validate-infra
- networking
created_at: 2026-06-03T21:58:50.567668623+00:00
updated_at: 2026-06-04T11:02:36.800418792+00:00
---

# Description

PROBLEM: validate runs spawn children (mtg server, python http.server, headless chromium, validate.sh) that orphan / double-fork (setsid) and ESCAPE process-group kills, lingering as zombies. These cause: port-collision desync false-positives (the All-Hallow's-Eve network-e2e H2 flake; mtg-r1osh lock-precheck false-starts) + resource contention across the 4 concurrent slots. Confirmed live 2026-06-03: 4 orphan procs present during the session. slot02 found that killpg/process-group reaping trips exit-144 with pipe stdout AND cannot catch setsid-escaping daemons -- process groups are the wrong tool.

FIX (the lightest; VERIFIED on the dev box 2026-06-03 -- systemd 255, cgroup v2 (cgroup2fs), user-delegated subtree user-<uid>.slice with controllers cpu/memory/pids + cgroup.kill present, NO ROOT, `systemd-run --user --scope true` succeeds):
Wrap each validate run in a transient systemd SCOPE (a cgroup):
  systemd-run --user --scope --unit=validate-<id> make validate
Kill the ENTIRE descendant tree atomically (incl. setsid/double-forked daemons):
  systemctl --user stop validate-<id>.scope        # or: systemctl --user kill -s SIGKILL validate-<id>.scope
A cgroup captures EVERY transitive descendant regardless of fork/setsid; killing the scope SIGKILLs the whole cgroup atomically. No PID-guessing, no /proc/cwd scanning, no exit-144 (a scope is just a cgroup boundary, transparent to stdout).

BARE-KERNEL ALTERNATIVE (no systemd; cgroup.kill confirmed present):
  CG=/sys/fs/cgroup/user.slice/user-$(id -u).slice/validate-<id>
  mkdir "$CG"; echo $$ > "$CG"/cgroup.procs; make validate; echo 1 > "$CG"/cgroup.kill   # atomic whole-subtree SIGKILL

INTEGRATION: wrap the validate entrypoint (scripts/validate.sh and/or per-step in scripts/validate_run.py) so each run/step is its own named scope; replace kill_zombie_processes.py's /proc-scanning + the lock-precheck with `systemctl --user stop 'validate-*.scope'` (or cgroup.kill). Robust, atomic, zero false-positives.

CI CAVEAT: GitHub runners may lack user-systemd -> use cgroup v2 cgroup.kill on a runner-created cgroup, OR keep slot02's per-step setsid + proc.wait(timeout) + SIGKILL (adequate on the ephemeral runner). systemd-run scope on the shared dev box (where zombies hurt); cgroup.kill/timeout in CI.

SUPERSEDES the cwd-scoped /proc-reaper idea (was folded into mtg-726): cgroup-scoping is strictly more robust (captures setsid escapees + atomic kill, no /proc race). Likely CLOSES mtg-r1osh + the All-Hallow's-Eve H2 port-collision class outright. Also subsumes slot03's Playwright-screenshot-flake mitigation context (the contention these zombies cause).

OWNER: validate-infra -> slot02 (validate-overhaul / mtg-726). Coordinate so it doesn't bolt onto the merge-critical mtg-717 hang-fix; this is the follow-up that makes validates reliably zombie-free.


== CI PROOF 2026-06-04 (build-once run 26942309916, network-redo shard) ==
The "CI CAVEAT: keep per-step setsid + proc.wait(timeout) + SIGKILL (adequate
on the ephemeral runner)" assumption is DISPROVEN. Observed: `network.landing`
(a playwright browser step) FAILED, then the shard sat SILENT for 56 minutes
(09:27:53 -> 10:23:07) until cancelled; GH runner cleanup then reported
"Terminate orphan process: pid (3499) (python3)" = the validate_run.py runner
itself, alive but wedged. ROOT CAUSE: the failed browser step left a
setsid/double-forked orphan (chromium/node) holding the inherited stdout pipe
fd -> no EOF -> the runner's daemon-reader / next-step proc.wait never returns
-> the ENTIRE shard hangs (the 600s DEFAULT_STEP_TIMEOUT never fires because
the block is in pipe-read/reader-join, not in the waited child). proc.wait
timeout + per-PID SIGKILL is therefore NOT sufficient even in CI: it cannot
reap setsid escapees and does not break a reader blocked on an orphan-held pipe.
=> The cgroup.kill per-step reaper (cgroup v2 on a runner-created cgroup, per
this issue's CI alternative) is REQUIRED, not optional, for CI too. This is the
merge gate for mtg-717 build-once: network-redo cannot merge until a failed/hung
step is guaranteed reaped so it can't wedge the shard. Pair with the
network.landing waitUntil:'domcontentloaded' robustness fix (see mtg-717
CHECKPOINT 2026-06-04). Priority stays P2; now also on the mtg-717 critical path.


== UPDATE 2026-06-04 (build-once @ 0d90b46c): per-step reaper IMPLEMENTED + corrected wedge root-cause + full isolation design ==

CORRECTED ROOT CAUSE of the network-redo wedge: it was NOT the orphan-holds-pipe
theory. It was a SCHEDULER LIVELOCK in validate_run.py Runner.run(): when a step
fails, self.stop=True; remaining steps whose deps SUCCEEDED (the prebuilt
build.mtg-release/wasm.bundle) are then neither launched (stop) nor counted by
_skipped() (no dep failed), so `done+skipped >= steps` is never reached and the
loop busy-waits forever. The orphan python3 in the GH cleanup trace WAS that
livelocked runner. FIXED at 0d90b46c (terminate when not running AND (self.stop
OR all-terminal)) — robust fail-fast, no wedge on ANY future step failure.

PER-STEP ORPHAN REAPER — IMPLEMENTED (0d90b46c), killpg variant (NOT cgroup yet):
each step now Popen(start_new_session=True) → its own process group (pgid==pid);
on completion/timeout the runner killpg(pgid, SIGKILL) reaps leaked mtg
server/http.server/chromium grandchildren. pgid captured right after Popen
(getpgid(pid) raises once proc.wait collects the leader). Guarded against the
runner's own pgrp — the historical exit-144 was killpg WITHOUT start_new_session
(child shared the runner's group → suicide); the new session makes it safe.
Verified locally: orphan grandchild SIGKILLed, no runner suicide, network.landing
PASS with zero orphans left. LIMITATION: killpg misses processes that setsid()
into a NEW session/group (true double-fork daemons). chromium/playwright/mtg
server do NOT setsid away, so killpg catches them in practice — but cgroup.kill
(below) is the strictly-stronger successor for setsid escapees.

FULL ISOLATION DESIGN (user-directed, follow-on — two orthogonal axes + a scope):
A) PROCESS containment (cgroup.kill): replace/augment the killpg reaper with a
   per-step transient cgroup; `echo 1 > cgroup.kill` atomically SIGKILLs the
   whole subtree INCLUDING setsid escapees. CI caveat: GH-hosted runners may
   lack user-systemd / unprivileged cgroup delegation — verify; fall back to the
   killpg reaper (already in place) there.
B) SELF-REEXEC WHOLE-RUN SCOPE (local hygiene default): at the top of
   validate_run.py, re-exec the runner under `systemd-run --user --scope
   --setenv=MTG_VALIDATE_IN_SCOPE=1 ...` UNLESS (MTG_VALIDATE_IN_SCOPE set →
   anti-recursion) OR ($CI/$GITHUB_ACTIONS set → CI runs direct) OR (--no-scope)
   OR (systemd-run/systemctl --user unavailable → graceful skip). Effect: every
   LOCAL validate self-isolates in a transient cgroup; all descendants incl.
   setsid escapees are reaped on exit → no orphan leak between runs/agents
   (kills the stale-lock + port-collision-false-positive class). Dev box HAS
   systemd --user + cgroup v2. NOTE: whole-run scope ALONE does NOT fix a
   mid-run wedge (the runner would hang inside the scope before exit-reap) — so
   the per-step reaper (A / current killpg) is still required. Both needed.
C) NETWORK NAMESPACE port-isolation (the bigger payoff): run each validate (or
   each network-e2e harness) under `unshare --net` (+ `ip link set lo up`
   inside) so each gets its OWN loopback / independent port space. PREVENTS the
   port-collision-desync-false-positive class at the ROOT (vs racing to reap
   orphans) AND lets concurrent validates run SAFELY IN PARALLEL — directly
   fixing the cross-slot contention that forced serialized validates and serving
   mtg-717's fast/all-cores goal. Privilege check: `unshare -rn` (user+net ns)
   unprivileged locally; CAP_NET_ADMIN on the runner (self-hosted ok?
   GH-hosted?). Harness must bind the namespaced loopback. cgroup (process kill)
   ⟂ netns (port isolation) = complementary axes of full isolation.

SCOPING: A (per-step reaper) + the landing domcontentloaded fix are on the
build-once 12/12 critical path and DONE @ 0d90b46c. B (self-reexec scope) and C
(netns) are the mtg-ibj22 follow-on design — do NOT balloon the 12/12 path.
