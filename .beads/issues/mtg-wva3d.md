---
title: 'mtg-6vzht follow-up: keyed battlefield reconciliation + per-zone dirty-check (element identity within changed zones)'
status: open
priority: 4
issue_type: task
created_at: 2026-06-04T05:40:02.894177091+00:00
updated_at: 2026-06-04T05:40:02.894177091+00:00
---

# Description

Follow-up to mtg-6vzht. v1 (whole-render skip) preserves element identity on UNCHANGED ticks for free, but a CHANGED tick still full-rebuilds all zones (tearing down <img>/card nodes, losing in-flight CSS transitions/focus/scroll for the cards that DIDN'T change in that tick). Two complementary upgrades, only worth building if that within-changed-tick churn proves user-visible (e.g. tap animations, hover/focus continuity): (1) keyed battlefield reconciliation — reuse card elements + their <img> by card_id, rebuild only the lightweight text/class/badge (a working implementation was built+reverted in the v1 PR to keep it minimal; recover from git history of fix-mtg-6vzht); (2) per-zone dirty-check — render-to-string + skip-if-unchanged per zone (hand/graveyard/logs/details), so during a log flurry only the log zone re-renders. The rendered string IS the change-signature → no forgotten-field risk. Modest perf (battlefield render is only ~0.84ms); primary value is UX/element-identity. Relates to mtg-i9bux (layout review).
