---
title: Launcher feature-parity restore + nav-regression fixes (lobby redo dropped launcher settings)
status: open
priority: 2
issue_type: task
created_at: 2026-06-02T01:22:07.914763361+00:00
updated_at: 2026-06-02T01:22:07.914763361+00:00
---

# Description

Parent: mtg-682. The lobby redo (mtg-35z3s) moved the launcher out of the game pages into launcher.html and made the game pages PURE renderers, but DROPPED several launcher settings/controls the old built-in launcher had. User found these in live playtest (2026-06-01). Web pages ONLY (index/launcher/deck_editor/native_game/tui_game .html + lobby_launcher.js + e2e). Native GUI stays launcher default.

REGRESSION AUDIT (old built-in launcher @70b58e18 / @c9e08025 vs current):
1. Card-image SOURCE PICKER dropped. Old launcher had #img-src-local / #img-src-scryfall / #img-src-gatherer checkboxes + 'Show Card Images' toggle. tui_game.html still has ImageSource.getEnabledSources() reading those NOW-DELETED checkboxes -> always returns [] AND showImages hardcoded false -> TUI NEVER renders card images (hard regression). native_game.html uses auto-cascade tui_get_image_urls + filterImageUrls(allow_local) so it degraded gracefully (images on, local gated).
2. Debug-tracing control dropped. set_log_level('trace') only fired on the LOCAL boot path (bootConfig.debug); BOTH network boot paths hardcode debug:false -> no way to get TRACE in a networked game.
3. Settings persistence (SETTINGS_KEY/localStorage) dropped from the launcher.
4. Back-to-lobby / inter-page links DROP sticky params (allow_local_img_load etc). index.html forwards the gate to launcher+solo-game links, but the deck_editor link did NOT, and game-page 'Back to lobby' is a static index.html with no params.
5. deck_editor had NO 'Back to Launcher' (lobby-only) and its 'Use in Lobby' writes mtg_lobby_deck_preselect that NOTHING reads (dead).

FIX PLAN:
- launcher.html: add Show-images toggle + image-source picker (Local gated by allow_local_img_load sticky, Scryfall, Gatherer) + Debug(TRACE) toggle; persist via localStorage; forward to game page via &images=/&img_src=csv/&debug=.
- lobby_launcher.js: extend buildRedirectUrl + add consumeGamePrefs() (images/img_src/debug) parsed once; add forwardStickyParams() helper. Merge prefs into ALL 3 boot paths in both game pages.
- tui_game.html: ImageSource.getEnabledSources() reads param-derived config not dead DOM; showImages/debug from bootConfig prefs.
- native_game.html: debug carried on network boots; img_src filter honored.
- deck_editor.html: add Back to Launcher (preserves game/role/pass/name/ws/selected deck + image/debug params); make Use-in-Lobby/back preserve sticky params.

GATE: make validate green (DIVERGED:0/Failed:0). Extend web e2e: image-source gate (Local hidden default, shown w/ ?allow_local_img_load=true), gate SURVIVES back-to-lobby round trip, deck-editor->Back to Launcher returns context intact.
