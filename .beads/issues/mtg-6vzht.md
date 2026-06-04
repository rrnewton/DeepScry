---
title: renderBattlefield full innerHTML rebuild on every network message — only re-render on actual battlefield change (debounce/diff)
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T02:45:24.423604266+00:00
updated_at: 2026-06-04T05:39:38.539553703+00:00
---

# Description

PERF / ARCHITECTURE follow-up split out from mtg-j1ka3 (the flickering-opponent-card bug). 

CONTEXT: mtg-j1ka3 fixed the user-VISIBLE flicker with a resolved-URL image MEMO (native_game.html) — once a card image loads, the working URL is served first on later renders so the recreated <img> paints from cache (no re-cascade, no flicker). That is the SURGICAL symptom fix and is shipped.

DEEPER ISSUE (this issue): renderBattlefield() does `container.innerHTML = ''` then rebuilds the ENTIRE battlefield DOM (all sections, all card tiles, all <img>s) on EVERY processed network message — updateUI is scheduled per-message via networkClient.onMessageProcessed -> setTimeout(updateUI, 30) (native_game.html ~line 1787). During an active game this destroys+recreates the whole battlefield subtree dozens of times/sec regardless of whether anything on the battlefield actually changed. This is gratuitous DOM-thrash: layout/paint churn, lost element identity (focus/scroll/animation state), and it's the mechanism that MADE the flicker possible in the first place. Same CLASS as mtg-m9znz (the card-details pane reset src to urls[0] on every 30ms updateUI tick).

PROPOSED FIX (separate from the memo): only re-render the battlefield when its content actually CHANGED — either (a) a cheap content signature per container (card ids + tapped + counters + P/T + attachments + selection + labels) compared before rebuild, skip if identical; OR (b) keyed DOM reconciliation that reuses existing card/<img> elements for cards still present and only adds/removes/updates changed tiles. (b) is more robust (preserves element identity, no signature-completeness risk) but a larger rewrite; (a) is cheaper but must capture EVERY field that affects output or it staleness-bugs. Evaluate both. Applies to renderBattlefield + likely renderHand/renderGraveyard (same innerHTML-rebuild pattern).

WHY SEPARATE: the memo makes the flicker impossible even with the thrash, so this is a perf/architecture cleanup, not a correctness blocker. Keeping it out of mtg-j1ka3 keeps that bug fix tight + bankable.

RELATED: mtg-i9bux (battlefield layout-engine first-principles review — flagged render oscillation / the Rust->GUI dataflow), mtg-m9znz (the prior same-class card-details tick fix), mtg-j1ka3 (the flicker bug whose memo this complements).

── MEASUREMENT + RE-SCOPE 2026-06-04 (slot04, fix-mtg-6vzht) ──
Measured the per-tick cost (headless heuristic game, ~11 permanents, 200 iters; debug/measure_render_cost.js) BEFORE choosing the fix — the premise that the innerHTML rebuild is the dominator was WRONG:
  wasm view-model serialize+JSON.parse : ~2.27 ms/tick
  all-zone DOM render (updateUI−serialize): ~2.0 ms/tick  (battlefield ALONE only ~0.84 ms)
  full updateUI                          : ~4.3 ms/tick
So serialize and render are ~even halves; keyed reconciliation of the battlefield is mostly an element-identity win, not a perf lever.
SKIP-RATE (debug/measure_skip_rate.js): 78/101 updateUI ticks were no-ops → ~77% skippable (nav-inflated; gameplay-only ~40%). So MOST ticks render identical output — the real lever is CHANGE-DETECTION (skip unchanged ticks), not how we render.

SHIPPED (v1, this PR): a BULLETPROOF whole-render skip in native_game.html updateUI — renderKey = view-model JSON ⊕ measured card-sizes (resize-sensitive); identical ⇒ skip ALL rendering (no parse/index-rebuild/innerHTML/reflow; identity preserved for free). Immune to the under-render trap because the key IS the full render input (no forgotten-field failure mode), needs NO wasm change, and does NOT touch slot01's reveal-apply path. Saves the ~2ms render half on every unchanged tick. Regression test web/test_render_skip.js (in validate-wasm-e2e-step): (B) a real change is NEVER skipped (0 stale-render mismatches over 20 advances — the safety-critical no-under-render property); (A) no-op nav ticks produce 0 battlefield DOM mutations vs 8 for real advances (skip fires). 

DEFERRED/FILED follow-ups (NOT in v1):
- O(1) serialize-skip (skip the ~2.27ms serialize on unchanged ticks too): needs a TRUE-superset wasm view-revision counter (action_count alone is NOT a superset — misses selection/log/prompt → stale-UI under-render). The counter would bump in the reveal-apply path that slot01 is ACTIVELY reworking (netarch-reveal-actionlog-unify) → COORDINATE with/defer to slot01. → filed mtg-<a>.
- slim-serialize (cut the ~2.27ms: lazy oracle_text/full-details only when the details pane is open; logs only new lines) — helps ALL ticks, slot01-safe. → filed mtg-<b>.
- keyed battlefield reconciliation (element identity WITHIN a changed zone — focus/scroll/in-flight CSS transitions) + per-zone dirty-check (hand/graveyard/logs via render-to-string skip). Built+reverted in this PR (kept v1 minimal); → filed mtg-<c>.
