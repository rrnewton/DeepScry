---
title: Flickering card on opponent battlefield (native web GUI) — likely image-onerror retry loop or render oscillation
status: open
priority: 2
issue_type: bug
created_at: 2026-06-04T01:58:06.299392660+00:00
updated_at: 2026-06-04T02:46:04.173848646+00:00
---

# Description

USER-REPORTED 2026-06-04 (live deepscry.net game): a card on the OPPONENT's battlefield FLICKERED, both players on native web GUI. Likely either an image-load RETRY LOOP (CardImageOverlay/image_overlay.rs onerror → retry → fail → flicker) or a re-render oscillation. POSSIBLE CONNECTION: slot04's CDN migration (fix-mtg-722) just replaced the api.scryfall-keyed onerror loop-guard in image_overlay.rs:330 with a host-agnostic 'tried-once' Cell<bool> flag — verify whether that already fixes this, or whether the flicker is a DIFFERENT loop (e.g. the boot-load table miss → Gatherer fallback re-fetching). Also consider the battlefield-layout re-render path (mtg-i9bux review). INVESTIGATE: reproduce (a card whose image 404s on the opponent's board), determine image-onerror-thrash vs render-loop, fix the loop so a failed image settles to a single placeholder/Gatherer fallback (no flicker). Repro context: live game ~01:46 2026-06-04, opponent's battlefield. Relates to mtg-i9bux (layout review) + slot04 image_overlay work.

STATUS 2026-06-04 (fix-mtg-j1ka3 @ e9605f7a): FIXED via a resolved-URL image MEMO in native_game.html — once a card image loads, the working URL is served first on later renders so the recreated <img> paints from cache (no re-cascade, no flicker); shared by the battlefield + card-details paths. Regression test web/test_image_flicker_memo.js (in validate-wasm-e2e-step), 9/9 green. Root cause = render oscillation: renderBattlefield rebuilds container.innerHTML on every 30ms network tick, recreating each <img> from urls[0] (local), which 404s for opponent cards (no local art) → cascade restarts each render → flicker. The memo is the SURGICAL flicker fix; the underlying DOM-thrash is tracked SEPARATELY in mtg-6vzht (linked mtg-i9bux + mtg-m9znz). MTG-rules-review N/A (UI-only).
