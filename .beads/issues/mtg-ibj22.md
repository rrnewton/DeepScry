---
title: cgroup-isolate validate runs (transient systemd scope) -- kill all descendants as a unit, fix zombie mtg-server/http.server/chromium orphans
status: open
priority: 2
issue_type: task
labels:
- validate-infra
- networking
created_at: 2026-06-03T21:58:50.567668623+00:00
updated_at: 2026-06-04T11:16:04.090191174+00:00
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


== ISOLATION SEQUENCING + PORT/NETNS DESIGN (user-directed 2026-06-04, follow-on; does NOT gate build-once 12/12) ==

Ordered plan (do in this order; each independently valuable):
1. NESTED CGROUP robustness FIRST.
   - DONE: per-step killpg reaper (start_new_session + scoped killpg on
     completion/timeout) @ 0d90b46c.
   - NEXT: (a) whole-run SELF-REEXEC scope — top of validate_run.py re-execs under
     `systemd-run --user --scope --setenv=MTG_VALIDATE_IN_SCOPE=1` unless the
     sentinel is set (anti-recursion) / $CI|$GITHUB_ACTIONS (CI runs direct) /
     --no-scope / systemd unavailable (graceful skip); (b) cgroup.kill successor
     to killpg for setsid escapees. Nesting: per-step cgroup inside the whole-run
     scope.
2. EPHEMERAL-PORT GUARD (closes the collision gap the reaper only cleans up after).
   - Replace the unguarded `PORT=$((17800 + RANDOM % 10000))` in
     bug_finding/network_vs_local_equivalence_e2e.sh (and any sibling that picks a
     random fixed port up front) with `:0` TRUE-EPHEMERAL binding — kernel-atomic
     assignment, NO TOCTOU. PREFERRED (least fragile). If a port must be known
     before launch, use bind-probe-retry instead. This is independent of cgroup
     and worth doing regardless of netns.
3. NETWORK NAMESPACE — CONDITIONAL EXTRA-INSURANCE ONLY (user explicitly wary of
   fragility; high bar to enable):
   - Add ONLY after cgroup isolation is robust.
   - Gate on an END-TO-END availability probe, NOT just `unshare -rn` exit 0:
     `unshare -rn` → `ip link set lo up` → bind a listener on 127.0.0.1:0 →
     CONNECT to it → full round-trip success, ALL under a short timeout (~5s).
     Only a complete round-trip enables netns. Probe ONCE and cache the verdict.
   - Clean fallback to cgroup-only on ANY failure/timeout. Failure mode MUST be
     "we don't use netns," NEVER "we broke validate."
   - ALL-OR-NOTHING per network-e2e step: server + AI + web-server + chromium all
     in the SAME namespace, or none — never half a step namespaced.
   - With netns active, ports can return to a fixed DEFAULT_PORT (each ns has its
     own port space → no cross-run collision possible). This is the payoff that
     also enables SAFE PARALLEL validates (kills the cross-slot contention that
     forced serialized validates tonight).
   - CI: verify CAP_NET_ADMIN / unprivileged user-ns availability per runner
     BEFORE enabling there (self-hosted likely ok; GitHub-hosted may lack it) —
     the same probe handles this (fails → cgroup-only).

cgroup (process containment) ⟂ netns (port-space isolation) ⟂ :0 ephemeral
(collision-free binding) — orthogonal layers; ship 1→2→3 in confidence order.
