---
title: Decide long-term card-image licensing posture (currently gated behind ?allow_local_img_load=true)
status: open
priority: 3
issue_type: task
created_at: 2026-05-27T23:51:59.450863639+00:00
updated_at: 2026-05-28T03:24:00.881544210+00:00
---

# Description

Locally-served card images at `web/images/**` have unclear licensing. As of the gate-local-card-images change, the `Local` option in the "Show Card Images" picker is removed from the default UX (native_game.html / tui_game.html) and only re-enabled when the URL has `?allow_local_img_load=true`.

The unlock is now STICKY within a browser tab/session (sticky-allow-local-img branch): the URL param is authoritative when present (and `?allow_local_img_load=false` can explicitly re-lock), otherwise the flag is inherited from sessionStorage. We deliberately use sessionStorage (not localStorage) so the unlock survives same-tab navigation (index -> game page) but resets on tab close; localStorage would let a stale flag silently re-enable local images across sessions, defeating the gate. We never read localStorage, so old localStorage from prior builds cannot bypass the gate. index.html also propagates `?allow_local_img_load=true` onto the Native/TUI/demo launcher hrefs and the create/join redirect, so the param stays visible in the URL bar and shareable. The landing-page UX e2e test (web/test_landing_page_ux.js) gained a scenario verifying (a) launcher hrefs carry the param, (b) sessionStorage (not localStorage) persistence, (c) same-tab nav inherits the unlock, and (d) a fresh session with stale localStorage stays locked (anti-bypass).

Follow-ups to decide:
1. Can we serve any subset of card art locally without infringing on WotC IP? (Scryfall has clearer terms; Gatherer is WotC's own.)
2. If yes, codify the subset (e.g. tokens-only, basics-only) and document the source license per directory.
3. If no, plan to delete `web/images/` from the deploy and remove the local fallback from `wasm/image_overlay.rs::tui_get_image_urls`.

Until decided, the URL-param unlock (now session-sticky) is the user-only escape hatch.
