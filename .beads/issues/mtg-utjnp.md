---
title: 'History surgery: strip web/screenshots blobs from integration history'
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T19:46:50.625174091+00:00
updated_at: 2026-05-28T19:50:03.764856758+00:00
---

# Description

Strip the accidentally-committed QA screenshot blobs (web/screenshots/landing_page_qa/, 11 PNGs ~9.2 MB) from integration HISTORY so fresh clones on new machines are fast (user priority 2026-05-28: "I don't want slow checkouts on new machines, so I want the repo surgery").

Facts: blobs entered at commit 33aaa3cc (~85 commits back from integration HEAD at decision time); already removed from the tree at 6aedfb9e + globally gitignored (*.png) + policy added in CLAUDE.md. But the blobs remain reachable in history.

Plan (user-approved force-push for THIS case, 2026-05-28):
1. Do it in the quiescent window AFTER the desync worktree lands (no in-flight branch off integration to orphan).
2. git filter-repo --path web/screenshots --invert-paths (or BFG) to rewrite integration.
3. Keep a LOCAL integration.old.<YYYY-MM-DD> branch as backup while verifying sanity (build/validate/log) -- user wants this kept around locally until confirmed sane.
4. Force-push the rewritten integration.
5. Prune any REMOTE tips that keep the old blob-containing commits reachable: merged feature branches (ci-split-test-jobs, flakiness-harness, fix-desync-vk4b7, etc.) and handle trunk (origin c213c485 -- rebase onto clean integration, else it keeps blobs alive). Without this, GitHub GC never frees the blobs.
6. Verify: fresh git clone size drop; main unaffected (~140 commits behind, predates 33aaa3cc).

Order within the window: surgery FIRST (crate rename + sweep then build on clean rewritten history). Sequence vs beads-renumber: renumber first, then surgery (surgery just rewrites the renumber commit too).

BASELINE (fresh clone, 2026-05-28, depth ~2389, git@github.com:rrnewton/DeepScry.git):
- pack received 36.25 MiB; .git dir 38M; 22370 objects.
- screenshots ~9.2 MB raw (PNGs, near-incompressible) ~= 24% of pack.
- POST-SURGERY TARGET: .git should drop from 38M toward ~29M. Verify with a fresh clone + du -sh .git after the rewrite + remote-tip prune.
