---
title: 'PRINCIPLED: generalize rewind re-materialization to carry ALL per-instance state via reveal-actionlog (not per-field reconstruction)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-05T17:36:16.670285632+00:00
updated_at: 2026-06-05T17:36:16.670285632+00:00
---

# Description

slot04 desync-review 2026-06-05 (no-band-aid direction). Root issue: re-materializing an opponent permanent on rewind uses the blank template and loses EVERY per-instance fact (tapped, controller, damage, counters, P/T bonus, summoning-sickness, attachments, chosen_color). Field-by-field reconstruction (tapped done, controller + tap-sites next) is whack-a-mole. Principled fix: carry the full per-instance state through the reveal-actionlog unification (mtg-o99ow) so NO field is lost — subsumes the controller + tap-site + unhashed-field gaps. Immediate prize blockers (7/11/19) may be fixed first, but this is the durable fix. Related: mtg-o99ow, mtg-677.
