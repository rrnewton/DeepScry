---
title: 'Desync tooling P3: --field-trace built-in both-sides action_count-stamped field-write trace'
status: open
priority: 3
issue_type: task
created_at: 2026-06-05T17:42:17.619212195+00:00
updated_at: 2026-06-05T17:42:17.619212195+00:00
---

# Description

Audit P3. First-class debug mode emitting a stamped line per field-write on BOTH server and client: [ac=864 seq=37] card49.tapped false->true (writer=resolve_attack_step). Kills the re-add-ad-hoc-println-every-bug cycle (task-25 archetype). Gated like --network-debug. Effort M-L. Related: mtg-o99ow.
