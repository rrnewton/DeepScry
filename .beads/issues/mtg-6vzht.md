---
title: renderBattlefield full innerHTML rebuild on every network message — only re-render on actual battlefield change (debounce/diff)
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T02:45:24.423604266+00:00
updated_at: 2026-06-04T02:45:24.423604266+00:00
---

# Description

PERF / ARCHITECTURE follow-up split out from mtg-j1ka3 (the flickering-opponent-card bug). 

CONTEXT: mtg-j1ka3 fixed the user-VISIBLE flicker with a resolved-URL image MEMO (native_game.html) — once a card image loads, the working URL is served first on later renders so the recreated <img> paints from cache (no re-cascade, no flicker). That is the SURGICAL symptom fix and is shipped.

DEEPER ISSUE (this issue): renderBattlefield() does `container.innerHTML = ''` then rebuilds the ENTIRE battlefield DOM (all sections, all card tiles, all <img>s) on EVERY processed network message — updateUI is scheduled per-message via networkClient.onMessageProcessed -> setTimeout(updateUI, 30) (native_game.html ~line 1787). During an active game this destroys+recreates the whole battlefield subtree dozens of times/sec regardless of whether anything on the battlefield actually changed. This is gratuitous DOM-thrash: layout/paint churn, lost element identity (focus/scroll/animation state), and it's the mechanism that MADE the flicker possible in the first place. Same CLASS as mtg-m9znz (the card-details pane reset src to urls[0] on every 30ms updateUI tick).

PROPOSED FIX (separate from the memo): only re-render the battlefield when its content actually CHANGED — either (a) a cheap content signature per container (card ids + tapped + counters + P/T + attachments + selection + labels) compared before rebuild, skip if identical; OR (b) keyed DOM reconciliation that reuses existing card/<img> elements for cards still present and only adds/removes/updates changed tiles. (b) is more robust (preserves element identity, no signature-completeness risk) but a larger rewrite; (a) is cheaper but must capture EVERY field that affects output or it staleness-bugs. Evaluate both. Applies to renderBattlefield + likely renderHand/renderGraveyard (same innerHTML-rebuild pattern).

WHY SEPARATE: the memo makes the flicker impossible even with the thrash, so this is a perf/architecture cleanup, not a correctness blocker. Keeping it out of mtg-j1ka3 keeps that bug fix tight + bankable.

RELATED: mtg-i9bux (battlefield layout-engine first-principles review — flagged render oscillation / the Rust->GUI dataflow), mtg-m9znz (the prior same-class card-details tick fix), mtg-j1ka3 (the flicker bug whose memo this complements).
