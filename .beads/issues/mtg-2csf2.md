---
title: 'Web: consolidate ?allow_local_img_load into ?advanced_options=true (img + multiplayer seed field); rename ''Random AI'' -> ''Random'''
status: closed
priority: 3
issue_type: feature
created_at: 2026-06-05T13:52:32.690333177+00:00
updated_at: 2026-06-05T16:43:49.606459797+00:00
---

# Description

DONE (web-only, slot01 web-ux-playtest-batch).

(1) ?advanced_options=true is now the single gate, superseding ?allow_local_img_load (kept as a backward-compatible ALIAS). It unlocks BOTH (a) the 'Load from DeepScry server' local-image source AND (b) the advanced multiplayer RNG-seed field (creator-only).
- Gate resolution updated in: web/launcher.html (resolveAdvancedOptions), web/solo_launcher.html, web/native_game.html + web/tui_game.html (resolveLocalImageAllowed), web/index.html. Each accepts advanced_options first, falls back to the allow_local_img_load alias, sticky in sessionStorage.
- Propagation: web/lobby_launcher.js — advanced_options added to STICKY_PARAM_KEYS; buildRedirectQuery emits BOTH advanced_options and allow_local_img_load. index.html launcher/redirect links + deck_editor.html back-links emit both. Gate-note text now says ?advanced_options=true.

(2) Multiplayer seed FIELD added to web/launcher.html (#advanced-seed-field), shown only to the CREATOR and only when advanced_options is on. It is UI-ONLY and deliberately NOT wired into the live launch: a network game needs the SAME controller seed on both clients, and the launcher cannot share a creator seed with the joiner without server support. Wiring it client-side-only would desync the joiner (fatal). Server threading filed as follow-up mtg-737vj; field carries a 'pending server support' note.

(3) RENAME 'Random AI' -> 'Random' (it is not AI): web/solo_launcher.html:172,180 (both options) and web/launcher.html controller radio. Other AI labels (Heuristic AI, Zero AI) unchanged.

Backward-compat verified: existing e2e tests pass ?allow_local_img_load=true/false and assert it on links — still honored + still emitted. All inline HTML scripts pass node --check. Full make validate pending (orchestrator-sequenced).

Follow-up: mtg-737vj (server-side multiplayer seed threading through CreateGame).
