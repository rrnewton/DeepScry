---
title: 'mtg-6vzht follow-up: O(1) serialize-skip via a true-superset wasm view-revision counter (coordinate w/ slot01)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T05:40:02.889009663+00:00
updated_at: 2026-06-04T05:44:28.844557875+00:00
---

# Description

Follow-up to mtg-6vzht v1 (whole-render skip). v1 still pays the ~2.27ms view-model serialize on every (incl. unchanged, ~77%) tick. To skip the serialize too, updateUI needs a CHEAP O(1) version signal it can read WITHOUT serializing. action_count (undo_log.len()) is NOT a guaranteed superset of visible changes (misses tui_select_card selection, log-only appends, prompt/choices updates) → an action_count-keyed skip would UNDER-render = stale game UI. A correct signal needs a wasm tui_view_revision() counter bumped on EVERY view-affecting WasmFancyTuiState mutation: undo_log advance + selection set/clear + prompt/choices set + the reveal/sync-apply path. CAUTION: the reveal-apply bump site is exactly the path slot01 is reworking in netarch-reveal-actionlog-unify — COORDINATE or DEFER until that lands. Test: revision bumps IFF the serialized model changes, for each known op. Value: saves ~2.27ms on the ~77% skipped ticks (roughly doubles v1's win there).

REFINEMENT (team-lead 2026-06-04): do NOT use one hand-maintained global counter — that has a forgotten-bump failure mode and a missed bump → stale UI that v1's bulletproof renderKey compare CANNOT catch (v2 skips BEFORE the serialize v1 would have compared). Make it STRUCTURALLY superset: derive the revision from PER-COMPONENT versions maintained at each mutation CHOKE POINT — action_count (game state) + selection-version + prompt/choices-version + log-len + the reveal-apply bump — so a NEW view input structurally needs its own version (can't be silently forgotten). Plus the 'revision bumps iff serialized model changes, per op' test. ORDERING: do slim-serialize (mtg-pio60) FIRST — if it gets per-tick serialize cheap enough, v1+slim captures most of v2's benefit WITHOUT the skip-correctness risk; pursue v2 only if slim isn't enough. The reveal-apply bump MUST wait until slot01's netarch 4a lands (it's rewriting exactly that path).
