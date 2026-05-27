---
title: Decide long-term card-image licensing posture (currently gated behind ?allow_local_img_load=true)
status: open
priority: 3
issue_type: task
created_at: 2026-05-27T23:51:59.450863639+00:00
updated_at: 2026-05-27T23:51:59.450863639+00:00
---

# Description

Locally-served card images at `web/images/**` have unclear licensing. As of the gate-local-card-images change, the `Local` option in the "Show Card Images" picker is removed from the default UX (native_game.html / tui_game.html) and only re-enabled when the URL has `?allow_local_img_load=true`.

Follow-ups to decide:
1. Can we serve any subset of card art locally without infringing on WotC IP? (Scryfall has clearer terms; Gatherer is WotC's own.)
2. If yes, codify the subset (e.g. tokens-only, basics-only) and document the source license per directory.
3. If no, plan to delete `web/images/` from the deploy and remove the local fallback from `wasm/image_overlay.rs::tui_get_image_urls`.

Until decided, the URL-param unlock is the user-only escape hatch.
