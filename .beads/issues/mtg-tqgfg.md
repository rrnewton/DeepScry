---
title: 'Task gardening: audit + update/close 263 open issues against current code; fix dependency links'
status: open
priority: 2
issue_type: task
created_at: 2026-06-01T13:04:57.735476324+00:00
updated_at: 2026-06-01T13:04:57.735476324+00:00
---

# Description

USER (2026-06-01): mb list -s open shows ~263 issues, many likely stale. A dedicated gardening agent should, by READING THE ACTUAL CODE: (1) close issues whose work is already done/obsolete (with a one-line reason citing the code); (2) update still-relevant issues so their text matches current code; (3) fix/add dependency links so the graph is correct. Authorized to run DIRECTLY on the primary checkout / integration branch (contra normal worktree policy — it's beads-only, no build).

SCOPE GUARDS (coordinator, to avoid conflicts with in-flight goal work): do NOT touch the issue clusters tied to the two ACTIVE goals or held items — leave these alone: (a) lobby/launcher REDO cluster (mtg-35z3s, khy7x, 1vwpd, tnsk7, dw9j3, 594, 595, vj714, 6ue2b, g67ye, 33fmb, zaqgj, nisrk, 4a1f5) — just curated, active; (b) netarch cluster (mtg-53okw, 610, 559, c9fuc, uzvu4, 614/PR#11); (c) the mono-black deck being actively worked (mtg-560 + its per-card issues: 399,510,557,515,408,537,543,405,485,511,496,521,501,497,542,529,389) and the deck-compat tracker mtg-pph0s/mtg-34; (d) the new official-collections issue mtg-nmbr1. Garden the OTHER ~200 (older per-card compat issues for already-swept decks, optimization, infra, testing, TUI/feature issues). When UNSURE whether something is done, do NOT close it — leave open + add a 'GARDENING: possibly-stale, needs human/code re-check: <why>' note. Closing must cite concrete code evidence. Do NOT run mb mb-migrate / renumber (separate ceremony I run later). Commit beads in coherent batches w/ clear messages, push to integration.
