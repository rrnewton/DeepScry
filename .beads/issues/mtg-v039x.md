---
title: 'mtg-6vzht follow-up: O(1) serialize-skip via a true-superset wasm view-revision counter (coordinate w/ slot01)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T05:40:02.889009663+00:00
updated_at: 2026-06-04T05:40:02.889009663+00:00
---

# Description

Follow-up to mtg-6vzht v1 (whole-render skip). v1 still pays the ~2.27ms view-model serialize on every (incl. unchanged, ~77%) tick. To skip the serialize too, updateUI needs a CHEAP O(1) version signal it can read WITHOUT serializing. action_count (undo_log.len()) is NOT a guaranteed superset of visible changes (misses tui_select_card selection, log-only appends, prompt/choices updates) → an action_count-keyed skip would UNDER-render = stale game UI. A correct signal needs a wasm tui_view_revision() counter bumped on EVERY view-affecting WasmFancyTuiState mutation: undo_log advance + selection set/clear + prompt/choices set + the reveal/sync-apply path. CAUTION: the reveal-apply bump site is exactly the path slot01 is reworking in netarch-reveal-actionlog-unify — COORDINATE or DEFER until that lands. Test: revision bumps IFF the serialized model changes, for each known op. Value: saves ~2.27ms on the ~77% skipped ticks (roughly doubles v1's win there).
