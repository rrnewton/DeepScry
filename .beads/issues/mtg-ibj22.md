---
title: cgroup-isolate validate runs (transient systemd scope) -- kill all descendants as a unit, fix zombie mtg-server/http.server/chromium orphans
status: open
priority: 2
issue_type: task
labels:
- validate-infra
- networking
created_at: 2026-06-03T21:58:50.567668623+00:00
updated_at: 2026-06-03T21:58:50.567668623+00:00
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
