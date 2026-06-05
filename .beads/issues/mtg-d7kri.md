---
title: Killing a make-validate's bash orphans the systemd scope's mtg server/connect children (port + /tmp contention)
status: open
priority: 3
issue_type: task
created_at: 2026-06-05T04:57:24.570264551+00:00
updated_at: 2026-06-05T04:57:24.570264551+00:00
---

# Description

Companion to mtg-ibj22 / mtg-zw363. SEPARATE from the /tmp-collision root cause (fixed in mtg-zw363): when a 'make validate' is KILLED mid-run (pkill on the bash, Ctrl-C, or a parent timeout) rather than allowed to exit normally, the transient 'systemd-run --user --scope validate-<pid>.scope' is NOT torn down by killing the bash — its grandchild 'mtg server'/'mtg connect' processes (spawned by the web GUI e2e / network legs, often via setsid which escapes killpg) ORPHAN and keep running. Observed 2026-06-05 during the mtg-u3dwj work: after pkill'ing a validate, leftover 'mtg server --port 18602 --password test_gui' + a grizzly_bears 'mtg connect' kept holding the port and contending for the global /tmp game-log path; a subsequent validate then bailed on a stale .validate.lock left by the killed run. Recovery today is manual: scripts/kill_zombie_processes.py (which DOES stop the leftover scope + clear the lock). FIX DIRECTION: on abnormal validate termination, ensure the scope is stopped (systemctl --user stop validate-<pid>.scope) so systemd atomically reaps all descendants; e.g. a trap/cleanup that stops the scope unit by name, and/or have the lock-file own a scope-stop. This is the process-isolation-teardown half; the /tmp-uniqueness half is mtg-zw363 (fixed). RELATED: mtg-ibj22, mtg-zw363, the 'isolate validates before concurrency' memory (setsid orphans escape killpg).
