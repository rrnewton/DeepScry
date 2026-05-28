---
title: 'History surgery: strip web/screenshots blobs from integration history'
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T19:46:50.625174091+00:00
updated_at: 2026-05-28T19:49:32.097053386+00:00
---

# Description



BASELINE (fresh clone, 2026-05-28, depth ~2389, git@github.com:rrnewton/DeepScry.git):
- pack received 36.25 MiB; .git dir 38M; 22370 objects.
- screenshots ~9.2 MB raw (PNGs, near-incompressible) ~= 24% of pack.
- POST-SURGERY TARGET: .git should drop from 38M toward ~29M. Verify with a fresh clone + du -sh .git after the rewrite + remote-tip prune.
